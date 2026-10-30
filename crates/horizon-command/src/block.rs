//! BlockStateParser: block states (`stone`, `oak_stairs[facing=up]{...}`)
//! and block predicates (`#logs[axis=y]`), with vanilla's suggestions.
use super::args::{BlockInput, Registries, read_identifier, suggest_resources};
use super::{CmdResult, CommandError, Reader, SuggestionsBuilder};

#[derive(Clone, Debug)]
enum Sugg {
    Nothing,
    Item,
    BlockIdOrTag,
    Tag,
    OpenVaguePropertiesOrNbt,
    OpenNbt,
    OpenPropertiesOrNbt,
    PropertyNameOrEnd,
    VaguePropertyNameOrEnd,
    PropertyName,
    VaguePropertyName,
    Equals,
    PropertyValues(String),
    NextPropertyOrEnd,
    VaguePropertyValue(String),
}

type Props = Vec<(String, Vec<String>)>;

pub struct BlockParser<'r, 'a> {
    reader: &'r mut Reader<'a>,
    reg: &'r Registries,
    for_testing: bool,
    allow_nbt: bool,
    id: String,
    /// The block's state definition (properties and values).
    definition: Option<&'r Props>,
    properties: Vec<(String, String)>,
    vague: Vec<(String, String)>,
    tag: Option<String>,
    nbt: Option<String>,
    sugg: Sugg,
}

fn is_int_property(values: &[String]) -> bool {
    !values.is_empty() && values.iter().all(|v| v.parse::<i32>().is_ok())
}
/// Property.getValue.
fn property_value(values: &[String], raw: &str) -> Option<String> {
    if is_int_property(values) {
        let v: i32 = super::args::java_parse_int(raw)?;
        let s = v.to_string();
        return values.contains(&s).then_some(s);
    }
    values.iter().any(|v| v == raw).then(|| raw.to_owned())
}
/// addSuggestions: every value (integers as integer suggestions).
fn add_values(builder: &mut SuggestionsBuilder, values: &[String]) {
    let ints = is_int_property(values);
    for v in values {
        if ints {
            builder.suggest_int(v.parse().unwrap_or(0));
        } else {
            builder.suggest(v.clone());
        }
    }
}

impl<'r, 'a> BlockParser<'r, 'a> {
    pub fn new(
        reader: &'r mut Reader<'a>,
        reg: &'r Registries,
        for_testing: bool,
        allow_nbt: bool,
    ) -> Self {
        Self {
            reader,
            reg,
            for_testing,
            allow_nbt,
            id: "minecraft:".into(),
            definition: None,
            properties: Vec::new(),
            vague: Vec::new(),
            tag: None,
            nbt: None,
            sugg: Sugg::Nothing,
        }
    }

    fn tag_members(&self) -> &'r [String] {
        match &self.tag {
            Some(t) => self
                .reg
                .block_tag_members
                .get(t)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            None => &[],
        }
    }
    fn block_props(&self, id: &str) -> &'r [(String, Vec<String>)] {
        self.reg.blocks.get(id).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn parse(&mut self) -> CmdResult<BlockInput> {
        self.sugg = if self.for_testing {
            Sugg::BlockIdOrTag
        } else {
            Sugg::Item
        };
        if self.reader.can_read() && self.reader.peek() == '#' {
            self.read_tag()?;
            self.sugg = Sugg::OpenVaguePropertiesOrNbt;
            if self.reader.can_read() && self.reader.peek() == '[' {
                self.read_vague_properties()?;
                self.sugg = Sugg::OpenNbt;
            }
        } else {
            self.read_block()?;
            self.sugg = Sugg::OpenPropertiesOrNbt;
            if self.reader.can_read() && self.reader.peek() == '[' {
                self.read_properties()?;
                self.sugg = Sugg::OpenNbt;
            }
        }
        if self.allow_nbt && self.reader.can_read() && self.reader.peek() == '{' {
            self.sugg = Sugg::Nothing;
            let start = self.reader.cursor;
            super::snbt::parse_compound_argument(self.reader)?;
            self.nbt = Some(self.reader.string[start..self.reader.cursor].to_owned());
        }
        let (id, properties) = match &self.tag {
            Some(t) => (t.clone(), self.vague.clone()),
            None => (self.id.clone(), self.properties.clone()),
        };
        Ok(BlockInput {
            id,
            properties,
            tag: self.tag.is_some(),
            nbt: self.nbt.clone(),
        })
    }

    fn read_block(&mut self) -> CmdResult<()> {
        let start = self.reader.cursor;
        self.id = read_identifier(self.reader)?;
        match self.reg.blocks.get(&self.id) {
            Some(props) => {
                self.definition = Some(props);
                Ok(())
            }
            None => {
                self.reader.cursor = start;
                Err(CommandError::tr_at(
                    "argument.block.id.invalid",
                    &[&self.id],
                    self.reader,
                ))
            }
        }
    }
    fn read_tag(&mut self) -> CmdResult<()> {
        if !self.for_testing {
            return Err(CommandError::tr_at(
                "argument.block.tag.disallowed",
                &[],
                self.reader,
            ));
        }
        let start = self.reader.cursor;
        self.reader.expect('#')?;
        self.sugg = Sugg::Tag;
        let id = read_identifier(self.reader)?;
        if !self.reg.tags("minecraft:block").contains(&id) {
            self.reader.cursor = start;
            return Err(CommandError::tr_at(
                "arguments.block.tag.unknown",
                &[&id],
                self.reader,
            ));
        }
        self.tag = Some(id);
        Ok(())
    }
    fn read_properties(&mut self) -> CmdResult<()> {
        self.reader.skip();
        self.sugg = Sugg::PropertyNameOrEnd;
        self.reader.skip_whitespace();
        let definition = self.definition.map(Vec::as_slice).unwrap_or(&[]);
        while self.reader.can_read() && self.reader.peek() != ']' {
            self.reader.skip_whitespace();
            let key_start = self.reader.cursor;
            let key = self.reader.read_string()?;
            let Some((_, values)) = definition.iter().find(|(n, _)| *n == key) else {
                self.reader.cursor = key_start;
                return Err(CommandError::tr_at(
                    "argument.block.property.unknown",
                    &[&self.id, &key],
                    self.reader,
                ));
            };
            if self.properties.iter().any(|(n, _)| *n == key) {
                self.reader.cursor = key_start;
                return Err(CommandError::tr_at(
                    "argument.block.property.duplicate",
                    &[&key, &self.id],
                    self.reader,
                ));
            }
            self.reader.skip_whitespace();
            self.sugg = Sugg::Equals;
            if !self.reader.can_read() || self.reader.peek() != '=' {
                return Err(CommandError::tr_at(
                    "argument.block.property.novalue",
                    &[&key, &self.id],
                    self.reader,
                ));
            }
            self.reader.skip();
            self.reader.skip_whitespace();
            self.sugg = Sugg::PropertyValues(key.clone());
            let start = self.reader.cursor;
            let raw = self.reader.read_string()?;
            match property_value(values, &raw) {
                Some(v) => self.properties.push((key, v)),
                None => {
                    self.reader.cursor = start;
                    return Err(CommandError::tr_at(
                        "argument.block.property.invalid",
                        &[&self.id, &raw, &key],
                        self.reader,
                    ));
                }
            }
            self.sugg = Sugg::NextPropertyOrEnd;
            self.reader.skip_whitespace();
            if self.reader.can_read() {
                if self.reader.peek() != ',' {
                    if self.reader.peek() != ']' {
                        return Err(CommandError::tr_at(
                            "argument.block.property.unclosed",
                            &[],
                            self.reader,
                        ));
                    }
                    break;
                }
                self.reader.skip();
                self.sugg = Sugg::PropertyName;
            }
        }
        if self.reader.can_read() {
            self.reader.skip();
            Ok(())
        } else {
            Err(CommandError::tr_at(
                "argument.block.property.unclosed",
                &[],
                self.reader,
            ))
        }
    }
    fn read_vague_properties(&mut self) -> CmdResult<()> {
        self.reader.skip();
        self.sugg = Sugg::VaguePropertyNameOrEnd;
        let mut value_start: Option<usize> = None;
        self.reader.skip_whitespace();
        while self.reader.can_read() && self.reader.peek() != ']' {
            self.reader.skip_whitespace();
            let key_start = self.reader.cursor;
            let key = self.reader.read_string()?;
            if self.vague.iter().any(|(n, _)| *n == key) {
                self.reader.cursor = key_start;
                return Err(CommandError::tr_at(
                    "argument.block.property.duplicate",
                    &[&key, &self.id],
                    self.reader,
                ));
            }
            self.reader.skip_whitespace();
            if !self.reader.can_read() || self.reader.peek() != '=' {
                self.reader.cursor = key_start;
                return Err(CommandError::tr_at(
                    "argument.block.property.novalue",
                    &[&key, &self.id],
                    self.reader,
                ));
            }
            self.reader.skip();
            self.reader.skip_whitespace();
            self.sugg = Sugg::VaguePropertyValue(key.clone());
            value_start = Some(self.reader.cursor);
            let value = self.reader.read_string()?;
            self.vague.push((key, value));
            self.reader.skip_whitespace();
            if self.reader.can_read() {
                value_start = None;
                if self.reader.peek() != ',' {
                    if self.reader.peek() != ']' {
                        return Err(CommandError::tr_at(
                            "argument.block.property.unclosed",
                            &[],
                            self.reader,
                        ));
                    }
                    break;
                }
                self.reader.skip();
                self.sugg = Sugg::VaguePropertyName;
            }
        }
        if self.reader.can_read() {
            self.reader.skip();
            Ok(())
        } else {
            if let Some(v) = value_start {
                self.reader.cursor = v;
            }
            Err(CommandError::tr_at(
                "argument.block.property.unclosed",
                &[],
                self.reader,
            ))
        }
    }

    fn has_block_entity(&self) -> bool {
        if self.tag.is_none() {
            return self.reg.block_entities.contains(&self.id);
        }
        self.tag_members()
            .iter()
            .any(|b| self.reg.block_entities.contains(b))
    }

    pub fn fill_suggestions(&self, builder: &mut SuggestionsBuilder) {
        let mut b = builder.offset(self.reader.cursor);
        let empty = b.remaining().is_empty();
        let definition = self.definition.map(Vec::as_slice).unwrap_or(&[]);
        match &self.sugg {
            Sugg::Nothing => {}
            Sugg::Item => {
                let ids: Vec<&String> = self.reg.blocks.keys().collect();
                suggest_resources(&mut b, ids, "");
            }
            Sugg::Tag => suggest_resources(&mut b, self.reg.tags("minecraft:block"), "#"),
            Sugg::BlockIdOrTag => {
                suggest_resources(&mut b, self.reg.tags("minecraft:block"), "#");
                let ids: Vec<&String> = self.reg.blocks.keys().collect();
                suggest_resources(&mut b, ids, "");
            }
            Sugg::OpenVaguePropertiesOrNbt => {
                if empty && self.tag.is_some() {
                    let members = self.tag_members();
                    if members.iter().any(|m| !self.block_props(m).is_empty()) {
                        b.suggest("[");
                    }
                    if members.iter().any(|m| self.reg.block_entities.contains(m)) {
                        b.suggest("{");
                    }
                }
            }
            Sugg::OpenNbt => {
                if empty && self.has_block_entity() {
                    b.suggest("{");
                }
            }
            Sugg::OpenPropertiesOrNbt => {
                if empty {
                    if !definition.is_empty() {
                        b.suggest("[");
                    }
                    if self.reg.block_entities.contains(&self.id) {
                        b.suggest("{");
                    }
                }
            }
            Sugg::PropertyNameOrEnd | Sugg::PropertyName => {
                if matches!(self.sugg, Sugg::PropertyNameOrEnd) && empty {
                    b.suggest("]");
                }
                let prefix = b.remaining_lower();
                for (name, _) in definition {
                    if !self.properties.iter().any(|(n, _)| n == name) && name.starts_with(&prefix)
                    {
                        b.suggest(format!("{name}="));
                    }
                }
            }
            Sugg::VaguePropertyNameOrEnd | Sugg::VaguePropertyName => {
                if matches!(self.sugg, Sugg::VaguePropertyNameOrEnd) && empty {
                    b.suggest("]");
                }
                let prefix = b.remaining_lower();
                for m in self.tag_members() {
                    for (name, _) in self.block_props(m) {
                        if !self.vague.iter().any(|(n, _)| n == name) && name.starts_with(&prefix) {
                            b.suggest(format!("{name}="));
                        }
                    }
                }
            }
            Sugg::Equals => {
                if empty {
                    b.suggest("=");
                }
            }
            Sugg::PropertyValues(key) => {
                if let Some((_, values)) = definition.iter().find(|(n, _)| n == key) {
                    add_values(&mut b, values);
                }
            }
            Sugg::NextPropertyOrEnd => {
                if empty {
                    b.suggest("]");
                }
                if empty && self.properties.len() < definition.len() {
                    b.suggest(",");
                }
            }
            Sugg::VaguePropertyValue(key) => {
                let mut more = false;
                for m in self.tag_members() {
                    let props = self.block_props(m);
                    if let Some((_, values)) = props.iter().find(|(n, _)| n == key) {
                        add_values(&mut b, values);
                    }
                    if !more {
                        more = props
                            .iter()
                            .any(|(n, _)| !self.vague.iter().any(|(v, _)| v == n));
                    }
                }
                if more {
                    b.suggest(",");
                }
                b.suggest("]");
            }
        }
        builder.add(b);
    }
}

/// BlockStateParser.parseForBlock / parseForTesting (the reader is rewound on errors).
pub fn parse(reader: &mut Reader, reg: &Registries, for_testing: bool) -> CmdResult<BlockInput> {
    let cursor = reader.cursor;
    let r = BlockParser::new(reader, reg, for_testing, true).parse();
    if r.is_err() {
        reader.cursor = cursor;
    }
    r
}
/// BlockStateParser.fillSuggestions.
pub fn suggest(builder: &mut SuggestionsBuilder, reg: &Registries, for_testing: bool) {
    let mut reader = Reader {
        string: builder.input,
        cursor: builder.start,
    };
    let mut parser = BlockParser::new(&mut reader, reg, for_testing, true);
    let _ = parser.parse();
    parser.fill_suggestions(builder);
}
