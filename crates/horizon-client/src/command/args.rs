//! Argument types (Brigadier's and Minecraft's `ArgumentType`s): parsing with
//! the vanilla error messages and completion suggestions.
use super::{CmdResult, CommandError, Context, Reader, SuggestionsBuilder};
use std::collections::BTreeMap;

/// Registry contents and source state used to validate and complete arguments.
#[derive(Clone, Debug, Default)]
pub struct Registries {
    /// Permission level of the command source.
    pub permission: u8,
    pub items: Vec<String>,
    pub item_tags: Vec<String>,
    /// Block id -> (property, allowed values).
    pub blocks: BTreeMap<String, Vec<(String, Vec<String>)>>,
    pub block_tags: Vec<String>,
    /// Entity types that `summon` can create.
    pub summonable: Vec<String>,
    /// Every entity type (selector `type=`).
    pub entity_types: Vec<String>,
    pub effects: Vec<String>,
    pub enchantments: Vec<String>,
    pub structures: Vec<String>,
    pub biomes: Vec<String>,
    pub dimensions: Vec<String>,
    pub clocks: Vec<String>,
    pub timelines: Vec<String>,
    pub time_markers: Vec<String>,
    pub advancements: Vec<String>,
    /// Game rule name -> integer-valued.
    pub game_rules: Vec<(String, bool)>,
    pub players: Vec<String>,
    /// The block the source is looking at (relevant coordinates).
    pub target_block: Option<[i32; 3]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    EntityType,
    Effect,
    Enchantment,
    Structure,
    Biome,
    Clock,
    Timeline,
}
impl Resource {
    pub fn registry(self) -> &'static str {
        match self {
            Resource::EntityType => "minecraft:entity_type",
            Resource::Effect => "minecraft:mob_effect",
            Resource::Enchantment => "minecraft:enchantment",
            Resource::Structure => "minecraft:worldgen/structure",
            Resource::Biome => "minecraft:worldgen/biome",
            Resource::Clock => "minecraft:world_clock",
            Resource::Timeline => "minecraft:timeline",
        }
    }
    fn values(self, reg: &Registries) -> &[String] {
        match self {
            Resource::EntityType => &reg.summonable,
            Resource::Effect => &reg.effects,
            Resource::Enchantment => &reg.enchantments,
            Resource::Structure => &reg.structures,
            Resource::Biome => &reg.biomes,
            Resource::Clock => &reg.clocks,
            Resource::Timeline => &reg.timelines,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ArgType {
    Bool,
    Integer {
        min: i32,
        max: i32,
    },
    Float {
        min: f32,
        max: f32,
    },
    Double {
        min: f64,
        max: f64,
    },
    Word,
    Str,
    Greedy,
    Entity {
        single: bool,
        players: bool,
    },
    BlockPos,
    Vec3 {
        center: bool,
    },
    Rotation,
    Angle,
    Item,
    ItemPredicate,
    BlockState,
    BlockPredicate,
    Resource(Resource),
    /// A plain identifier; suggestions come from the named list.
    Id(&'static str),
    Time {
        min: i32,
    },
    GameMode,
    Message,
    Dimension,
    Anchor,
    /// SwizzleArgument: a set of `x`, `y` and `z` (kept as text).
    Swizzle,
    /// ComponentArgument: an SNBT/JSON text component (kept as JSON text).
    Component,
    /// CompoundTagArgument (kept as SNBT text).
    Nbt,
}
pub fn integer(min: i32) -> ArgType {
    ArgType::Integer { min, max: i32::MAX }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coord {
    pub relative: bool,
    pub value: f64,
}
impl Coord {
    pub fn get(self, base: f64) -> f64 {
        if self.relative {
            base + self.value
        } else {
            self.value
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Coordinates {
    World([Coord; 3]),
    Local([f64; 3]),
}
impl Coordinates {
    /// WorldCoordinates/LocalCoordinates.getPosition from the source position
    /// (feet) and rotation (yaw, pitch in degrees).
    pub fn position(&self, pos: [f64; 3], rot: [f32; 2]) -> [f64; 3] {
        match self {
            Coordinates::World(c) => [c[0].get(pos[0]), c[1].get(pos[1]), c[2].get(pos[2])],
            Coordinates::Local([left, up, forwards]) => {
                let d = std::f32::consts::PI / 180.0;
                let (yaw, pitch) = (rot[0], rot[1]);
                let cos_y = ((yaw + 90.0) * d).cos();
                let sin_y = ((yaw + 90.0) * d).sin();
                let cos_p = (-pitch * d).cos();
                let sin_p = (-pitch * d).sin();
                let cos_p2 = ((-pitch + 90.0) * d).cos();
                let sin_p2 = ((-pitch + 90.0) * d).sin();
                let f = [
                    f64::from(cos_y * cos_p),
                    f64::from(sin_p),
                    f64::from(sin_y * cos_p),
                ];
                let u = [
                    f64::from(cos_y * cos_p2),
                    f64::from(sin_p2),
                    f64::from(sin_y * cos_p2),
                ];
                // left = -(forward x up)
                let l = [
                    -(f[1] * u[2] - f[2] * u[1]),
                    -(f[2] * u[0] - f[0] * u[2]),
                    -(f[0] * u[1] - f[1] * u[0]),
                ];
                [
                    pos[0] + f[0] * forwards + u[0] * up + l[0] * left,
                    pos[1] + f[1] * forwards + u[1] * up + l[1] * left,
                    pos[2] + f[2] * forwards + u[2] * up + l[2] * left,
                ]
            }
        }
    }
    pub fn block(&self, pos: [f64; 3], rot: [f32; 2]) -> [i32; 3] {
        let p = self.position(pos, rot);
        [
            p[0].floor() as i32,
            p[1].floor() as i32,
            p[2].floor() as i32,
        ]
    }
    /// RotationArgument: (yaw, pitch) from the source rotation.
    pub fn rotation(&self, rot: [f32; 2]) -> [f32; 2] {
        match self {
            Coordinates::World(c) => [
                c[0].get(f64::from(rot[0])) as f32,
                c[1].get(f64::from(rot[1])) as f32,
            ],
            Coordinates::Local(_) => rot,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Arbitrary,
    Nearest,
    Furthest,
    Random,
}
pub type DoubleRange = (Option<f64>, Option<f64>);
pub type IntRange = (Option<i32>, Option<i32>);
/// A parsed entity selector (EntitySelector) or player name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selector {
    pub player_name: Option<String>,
    /// 'p', 'a', 'r', 's', 'e', 'n'.
    pub kind: char,
    pub limit: usize,
    pub sort: Sort,
    pub include_entities: bool,
    pub types: Vec<(String, bool)>,
    pub names: Vec<(String, bool)>,
    pub tags: Vec<(String, bool)>,
    pub gamemodes: Vec<(String, bool)>,
    pub distance: Option<DoubleRange>,
    pub level: Option<IntRange>,
    pub x_rotation: Option<DoubleRange>,
    pub y_rotation: Option<DoubleRange>,
    pub pos: [Option<f64>; 3],
    pub delta: [Option<f64>; 3],
}
impl Selector {
    pub fn is_self(&self) -> bool {
        self.kind == 's'
    }
    pub fn single(&self) -> bool {
        self.limit <= 1
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct BlockInput {
    pub id: String,
    pub properties: Vec<(String, String)>,
    /// `#tag` for block predicates.
    pub tag: bool,
    pub nbt: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ItemInput {
    pub id: String,
    pub tag: bool,
    pub any: bool,
    pub components: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameMode {
    Survival,
    Creative,
    Adventure,
    Spectator,
}
impl GameMode {
    pub const ALL: [GameMode; 4] = [
        GameMode::Survival,
        GameMode::Creative,
        GameMode::Adventure,
        GameMode::Spectator,
    ];
    pub fn name(self) -> &'static str {
        match self {
            GameMode::Survival => "survival",
            GameMode::Creative => "creative",
            GameMode::Adventure => "adventure",
            GameMode::Spectator => "spectator",
        }
    }
    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == s)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Bool(bool),
    Int(i32),
    Float(f32),
    Double(f64),
    Str(String),
    Entity(Selector),
    Coords(Coordinates),
    Angle(Coord),
    Item(ItemInput),
    Block(BlockInput),
    Resource(String),
    GameMode(GameMode),
    /// `eyes` (true) or `feet`.
    Anchor(bool),
}

fn is_id_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.' | ':' | '/')
}
/// Identifier.read: the longest run of identifier characters, normalised to
/// the `minecraft` namespace.
pub fn read_id(reader: &mut Reader) -> CmdResult<String> {
    let start = reader.cursor;
    while reader.can_read() && is_id_char(reader.peek()) {
        reader.skip();
    }
    let raw = &reader.string[start..reader.cursor];
    let valid = !raw.is_empty()
        && raw.matches(':').count() <= 1
        && !raw.ends_with(':')
        && !raw.split(':').next().unwrap_or("").contains('/');
    if !valid {
        reader.cursor = start;
        return Err(CommandError::tr_at("argument.id.invalid", &[], reader));
    }
    Ok(normalize(raw))
}
pub fn normalize(id: &str) -> String {
    match id.split_once(':') {
        Some(("", path)) => format!("minecraft:{path}"),
        Some(_) => id.to_owned(),
        None => format!("minecraft:{id}"),
    }
}
/// SharedSuggestionProvider.matchesSubStr: `input` is a prefix of
/// `candidate` or of a part after an underscore.
pub fn matches_sub_str(input: &str, candidate: &str) -> bool {
    let mut index = 0;
    loop {
        if candidate[index..].starts_with(input) {
            return true;
        }
        match candidate[index..].find('_') {
            Some(i) => index += i + 1,
            None => return false,
        }
    }
}
/// SharedSuggestionProvider.suggestResource.
pub fn suggest_resources<'a>(
    builder: &mut SuggestionsBuilder,
    ids: impl IntoIterator<Item = &'a String>,
    prefix: &str,
) {
    let remaining = builder.remaining_lower();
    let remaining = remaining
        .strip_prefix(prefix)
        .unwrap_or(&remaining)
        .to_owned();
    for id in ids {
        let matched = if remaining.contains(':') {
            matches_sub_str(&remaining, id)
        } else {
            let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
            matches_sub_str(&remaining, ns)
                || (ns == "minecraft" && matches_sub_str(&remaining, path))
        };
        if matched {
            builder.suggest(format!("{prefix}{id}"));
        }
    }
}
fn range_double(reader: &mut Reader) -> CmdResult<DoubleRange> {
    let (a, b) = read_range(reader)?;
    let parse = |s: &str, reader: &Reader| {
        s.parse::<f64>()
            .map_err(|_| CommandError::tr_at("parsing.double.invalid", &[&s], reader))
    };
    let lo = a.map(|s| parse(&s, reader)).transpose()?;
    let hi = b.map(|s| parse(&s, reader)).transpose()?;
    if let (Some(l), Some(h)) = (lo, hi)
        && l > h
    {
        return Err(CommandError::tr_at("argument.range.swapped", &[], reader));
    }
    Ok((lo, hi))
}
fn range_int(reader: &mut Reader) -> CmdResult<IntRange> {
    let (a, b) = read_range(reader)?;
    let parse = |s: &str, reader: &Reader| {
        if s.contains('.') {
            return Err(CommandError::tr_at("argument.range.ints", &[], reader));
        }
        s.parse::<i32>()
            .map_err(|_| CommandError::tr_at("parsing.int.invalid", &[&s], reader))
    };
    let lo = a.map(|s| parse(&s, reader)).transpose()?;
    let hi = b.map(|s| parse(&s, reader)).transpose()?;
    if let (Some(l), Some(h)) = (lo, hi)
        && l > h
    {
        return Err(CommandError::tr_at("argument.range.swapped", &[], reader));
    }
    Ok((lo, hi))
}
/// MinMaxBounds: `a`, `a..`, `..b` or `a..b`.
fn read_range(reader: &mut Reader) -> CmdResult<(Option<String>, Option<String>)> {
    if !reader.can_read() {
        return Err(CommandError::tr_at("argument.range.empty", &[], reader));
    }
    let number = |reader: &mut Reader| {
        let start = reader.cursor;
        while reader.can_read()
            && (reader.peek().is_ascii_digit()
                || reader.peek() == '-'
                || (reader.peek() == '.' && reader.peek_at(1) != '.'))
        {
            reader.skip();
        }
        let s = &reader.string[start..reader.cursor];
        (!s.is_empty()).then(|| s.to_owned())
    };
    let lo = number(reader);
    let hi = if reader.remaining().starts_with("..") {
        reader.skip();
        reader.skip();
        number(reader)
    } else {
        lo.clone()
    };
    if lo.is_none() && hi.is_none() {
        return Err(CommandError::tr_at("argument.range.empty", &[], reader));
    }
    Ok((lo, hi))
}
fn parse_coord_int(reader: &mut Reader) -> CmdResult<Coord> {
    if reader.can_read() && reader.peek() == '^' {
        return Err(CommandError::tr_at("argument.pos.mixed", &[], reader));
    }
    if !reader.can_read() {
        return Err(CommandError::tr_at("argument.pos.missing.int", &[], reader));
    }
    let relative = reader.peek() == '~';
    if relative {
        reader.skip();
    }
    let value = if reader.can_read() && reader.peek() != ' ' {
        if relative {
            reader.read_double()?
        } else {
            f64::from(reader.read_int()?)
        }
    } else {
        0.0
    };
    Ok(Coord { relative, value })
}
fn parse_coord_double(reader: &mut Reader, center: bool) -> CmdResult<Coord> {
    if reader.can_read() && reader.peek() == '^' {
        return Err(CommandError::tr_at("argument.pos.mixed", &[], reader));
    }
    if !reader.can_read() {
        return Err(CommandError::tr_at(
            "argument.pos.missing.double",
            &[],
            reader,
        ));
    }
    let relative = reader.peek() == '~';
    if relative {
        reader.skip();
    }
    let start = reader.cursor;
    let mut value = if reader.can_read() && reader.peek() != ' ' {
        reader.read_double()?
    } else {
        0.0
    };
    let number = &reader.string[start..reader.cursor];
    if relative && number.is_empty() {
        return Ok(Coord {
            relative,
            value: 0.0,
        });
    }
    if !number.contains('.') && !relative && center {
        value += 0.5;
    }
    Ok(Coord { relative, value })
}
fn parse_local(reader: &mut Reader) -> CmdResult<Coordinates> {
    let start = reader.cursor;
    let mut v = [0.0; 3];
    for (i, slot) in v.iter_mut().enumerate() {
        if i > 0 {
            if !reader.can_read() || reader.peek() != ' ' {
                reader.cursor = start;
                return Err(CommandError::tr_at(
                    "argument.pos3d.incomplete",
                    &[],
                    reader,
                ));
            }
            reader.skip();
        }
        if !reader.can_read() || reader.peek() != '^' {
            reader.cursor = start;
            return Err(CommandError::tr_at("argument.pos.mixed", &[], reader));
        }
        reader.skip();
        *slot = if reader.can_read() && reader.peek() != ' ' {
            reader.read_double()?
        } else {
            0.0
        };
    }
    Ok(Coordinates::Local(v))
}
fn parse_world(reader: &mut Reader, block: bool, center: bool) -> CmdResult<Coordinates> {
    let start = reader.cursor;
    let mut c = [Coord {
        relative: false,
        value: 0.0,
    }; 3];
    for (i, slot) in c.iter_mut().enumerate() {
        if i > 0 {
            if !reader.can_read() || reader.peek() != ' ' {
                reader.cursor = start;
                return Err(CommandError::tr_at(
                    "argument.pos3d.incomplete",
                    &[],
                    reader,
                ));
            }
            reader.skip();
        }
        *slot = if block {
            parse_coord_int(reader)?
        } else {
            parse_coord_double(reader, center && i != 1)?
        };
    }
    Ok(Coordinates::World(c))
}
fn suggest_coordinates(builder: &mut SuggestionsBuilder, reg: &Registries) {
    let remaining = builder.remaining().to_owned();
    let coords: Vec<[String; 3]> = if remaining.starts_with('^') {
        vec![["^".into(), "^".into(), "^".into()]]
    } else if let Some(t) = reg.target_block {
        vec![[t[0].to_string(), t[1].to_string(), t[2].to_string()]]
    } else {
        vec![["~".into(), "~".into(), "~".into()]]
    };
    if remaining.is_empty() {
        for c in &coords {
            builder.suggest(c[0].clone());
            builder.suggest(format!("{} {}", c[0], c[1]));
            builder.suggest(format!("{} {} {}", c[0], c[1], c[2]));
        }
    } else {
        let parts: Vec<&str> = remaining.split(' ').collect();
        for c in &coords {
            match parts.len() {
                1 => {
                    builder.suggest(format!("{} {}", parts[0], c[1]));
                    builder.suggest(format!("{} {} {}", parts[0], c[1], c[2]));
                }
                2 => builder.suggest(format!("{} {} {}", parts[0], parts[1], c[2])),
                _ => {}
            }
        }
    }
}

const SELECTORS: [(&str, &str); 6] = [
    ("@a", "argument.entity.selector.allPlayers"),
    ("@e", "argument.entity.selector.allEntities"),
    ("@n", "argument.entity.selector.nearestEntity"),
    ("@p", "argument.entity.selector.nearestPlayer"),
    ("@r", "argument.entity.selector.randomPlayer"),
    ("@s", "argument.entity.selector.self"),
];
const OPTIONS: [&str; 16] = [
    "name",
    "distance",
    "level",
    "x",
    "y",
    "z",
    "dx",
    "dy",
    "dz",
    "x_rotation",
    "y_rotation",
    "limit",
    "sort",
    "gamemode",
    "tag",
    "type",
];
fn parse_selector(reader: &mut Reader, reg: &Registries) -> CmdResult<Selector> {
    let start = reader.cursor;
    if !(reader.can_read() && reader.peek() == '@') {
        // EntitySelectorParser.parseNameOrUUID.
        let name = reader.read_string()?;
        if name.is_empty() || name.chars().count() > 16 {
            reader.cursor = start;
            return Err(CommandError::tr_at("argument.entity.invalid", &[], reader));
        }
        return Ok(Selector {
            player_name: Some(name),
            kind: '\0',
            limit: 1,
            include_entities: false,
            ..Selector::default()
        });
    }
    reader.skip();
    if !reader.can_read() {
        return Err(CommandError::tr_at(
            "argument.entity.selector.missing",
            &[],
            reader,
        ));
    }
    let kind = reader.read();
    let mut s = Selector {
        kind,
        ..Selector::default()
    };
    match kind {
        'p' => {
            s.limit = 1;
            s.sort = Sort::Nearest;
            s.types.push(("minecraft:player".into(), false));
        }
        'a' => {
            s.limit = usize::MAX;
            s.types.push(("minecraft:player".into(), false));
        }
        'r' => {
            s.limit = 1;
            s.sort = Sort::Random;
            s.types.push(("minecraft:player".into(), false));
        }
        's' => {
            s.limit = 1;
            s.include_entities = true;
        }
        'e' => {
            s.limit = usize::MAX;
            s.include_entities = true;
        }
        'n' => {
            s.limit = 1;
            s.sort = Sort::Nearest;
            s.include_entities = true;
        }
        other => {
            reader.cursor = start;
            return Err(CommandError::tr_at(
                "argument.entity.selector.unknown",
                &[&format!("@{other}")],
                reader,
            ));
        }
    }
    if reader.can_read() && reader.peek() == '[' {
        reader.skip();
        reader.skip_whitespace();
        while reader.can_read() && reader.peek() != ']' {
            reader.skip_whitespace();
            let option_start = reader.cursor;
            let option = reader.read_string()?;
            if !OPTIONS.contains(&option.as_str()) {
                reader.cursor = option_start;
                return Err(CommandError::tr_at(
                    "argument.entity.options.unknown",
                    &[&option],
                    reader,
                ));
            }
            reader.skip_whitespace();
            if !reader.can_read() || reader.peek() != '=' {
                reader.cursor = option_start;
                return Err(CommandError::tr_at(
                    "argument.entity.options.valueless",
                    &[&option],
                    reader,
                ));
            }
            reader.skip();
            reader.skip_whitespace();
            let negated = |reader: &mut Reader| {
                reader.skip_whitespace();
                if reader.can_read() && reader.peek() == '!' {
                    reader.skip();
                    reader.skip_whitespace();
                    true
                } else {
                    false
                }
            };
            match option.as_str() {
                "name" => {
                    let neg = negated(reader);
                    let v = reader.read_string()?;
                    s.names.push((v, neg));
                }
                "distance" => {
                    let r = range_double(reader)?;
                    if r.0.is_some_and(|v| v < 0.0) || r.1.is_some_and(|v| v < 0.0) {
                        reader.cursor = option_start;
                        return Err(CommandError::tr_at(
                            "argument.entity.options.distance.negative",
                            &[],
                            reader,
                        ));
                    }
                    s.distance = Some(r);
                    if s.kind != 's' {
                        s.sort = if s.sort == Sort::Arbitrary {
                            Sort::Arbitrary
                        } else {
                            s.sort
                        };
                    }
                }
                "level" => {
                    let r = range_int(reader)?;
                    if r.0.is_some_and(|v| v < 0) || r.1.is_some_and(|v| v < 0) {
                        reader.cursor = option_start;
                        return Err(CommandError::tr_at(
                            "argument.entity.options.level.negative",
                            &[],
                            reader,
                        ));
                    }
                    s.level = Some(r);
                    s.include_entities = false;
                }
                "x" | "y" | "z" => {
                    let i = usize::from(option.as_bytes()[0] - b'x');
                    s.pos[i] = Some(reader.read_double()?);
                }
                "dx" | "dy" | "dz" => {
                    let i = usize::from(option.as_bytes()[1] - b'x');
                    s.delta[i] = Some(reader.read_double()?);
                }
                "x_rotation" => s.x_rotation = Some(range_double(reader)?),
                "y_rotation" => s.y_rotation = Some(range_double(reader)?),
                "limit" => {
                    let at = reader.cursor;
                    let n = reader.read_int()?;
                    if n < 1 {
                        reader.cursor = at;
                        return Err(CommandError::tr_at(
                            "argument.entity.options.limit.toosmall",
                            &[],
                            reader,
                        ));
                    }
                    s.limit = n as usize;
                }
                "sort" => {
                    let at = reader.cursor;
                    let v = reader.read_unquoted_string();
                    s.sort = match v {
                        "nearest" => Sort::Nearest,
                        "furthest" => Sort::Furthest,
                        "random" => Sort::Random,
                        "arbitrary" => Sort::Arbitrary,
                        _ => {
                            reader.cursor = at;
                            return Err(CommandError::tr_at(
                                "argument.entity.options.sort.irreversible",
                                &[&v],
                                reader,
                            ));
                        }
                    };
                }
                "gamemode" => {
                    let neg = negated(reader);
                    let at = reader.cursor;
                    let v = reader.read_unquoted_string().to_owned();
                    if GameMode::from_name(&v).is_none() {
                        reader.cursor = at;
                        return Err(CommandError::tr_at(
                            "argument.entity.options.mode.invalid",
                            &[&v],
                            reader,
                        ));
                    }
                    s.gamemodes.push((v, neg));
                    s.include_entities = false;
                }
                "tag" => {
                    let neg = negated(reader);
                    let v = reader.read_unquoted_string().to_owned();
                    s.tags.push((v, neg));
                }
                "type" => {
                    let neg = negated(reader);
                    let at = reader.cursor;
                    let tag = reader.can_read() && reader.peek() == '#';
                    if tag {
                        reader.skip();
                    }
                    let id = read_id(reader)?;
                    if !tag && !reg.entity_types.is_empty() && !reg.entity_types.contains(&id) {
                        reader.cursor = at;
                        return Err(CommandError::tr_at(
                            "argument.entity.options.type.invalid",
                            &[&id],
                            reader,
                        ));
                    }
                    let id = if tag { format!("#{id}") } else { id };
                    if s.kind == 'p' || s.kind == 'a' || s.kind == 'r' {
                        s.types.clear();
                    }
                    if id == "minecraft:player" && !neg {
                        s.include_entities = false;
                    }
                    s.types.push((id, neg));
                }
                _ => {}
            }
            reader.skip_whitespace();
            if reader.can_read() && reader.peek() == ',' {
                reader.skip();
                continue;
            }
            if !reader.can_read() || reader.peek() != ']' {
                return Err(CommandError::tr_at(
                    "argument.entity.options.unterminated",
                    &[],
                    reader,
                ));
            }
        }
        if !reader.can_read() {
            return Err(CommandError::tr_at(
                "argument.entity.options.unterminated",
                &[],
                reader,
            ));
        }
        reader.skip();
    }
    Ok(s)
}
fn suggest_selector(builder: &mut SuggestionsBuilder, reg: &Registries) {
    let remaining = builder.remaining().to_owned();
    // Inside the option list: complete option names and values.
    if let Some(open) = remaining.find('[') {
        let inside = &remaining[open + 1..];
        let part_start = inside.rfind(',').map_or(0, |i| i + 1);
        let part = &inside[part_start..];
        let base = builder.start + open + 1 + part_start;
        if let Some((name, value)) = part.split_once('=') {
            let value_start = base + name.len() + 1;
            let mut b = builder.offset(value_start);
            let value = value.trim_start_matches('!');
            let neg = part[name.len() + 1..].starts_with('!');
            let mut b2 = builder.offset(value_start + usize::from(neg));
            let name = name.trim();
            let complete = match name {
                "type" => {
                    let v = value.trim_start_matches('#');
                    !v.is_empty()
                        && (reg.entity_types.contains(&normalize(v)) || value.starts_with('#'))
                }
                "sort" => ["nearest", "furthest", "random", "arbitrary"].contains(&value),
                "gamemode" => GameMode::from_name(value).is_some(),
                _ => !value.is_empty(),
            };
            if complete {
                let mut end = builder.offset(builder.input.len());
                end.suggest(",");
                end.suggest("]");
                builder.out.extend(end.out);
                return;
            }
            match name {
                "type" => {
                    let types: Vec<String> = reg.entity_types.clone();
                    suggest_resources(&mut b2, &types, "");
                    if !neg && value.is_empty() {
                        for t in &types {
                            b.suggest(format!("!{t}"));
                        }
                    }
                }
                "sort" => b2.suggest_matching(["nearest", "furthest", "random", "arbitrary"]),
                "gamemode" => b2.suggest_matching(GameMode::ALL.map(GameMode::name)),
                _ => {}
            }
            builder.out.extend(b.out);
            builder.out.extend(b2.out);
        } else {
            let mut b = builder.offset(base);
            let lower = part.trim_start().to_lowercase();
            for o in OPTIONS {
                if o.starts_with(&lower) {
                    b.suggest_tooltip(
                        format!("{o}="),
                        crate::lang::get(&format!("argument.entity.options.{o}.description")),
                    );
                }
            }
            if part.is_empty() {
                b.suggest("]");
            }
            builder.out.extend(b.out);
        }
        return;
    }
    if remaining.len() == 2 && remaining.starts_with('@') {
        let mut b = builder.offset(builder.input.len());
        b.suggest("[");
        builder.out.extend(b.out);
        return;
    }
    let lower = builder.remaining_lower();
    for (s, tip) in SELECTORS {
        if s.starts_with(&lower) {
            builder.suggest_tooltip(s, crate::lang::get(tip));
        }
    }
    let players = reg.players.clone();
    builder.suggest_matching(players.iter().map(String::as_str));
}
fn parse_block(reader: &mut Reader, reg: &Registries, tags: bool) -> CmdResult<BlockInput> {
    let start = reader.cursor;
    let tag = tags && reader.can_read() && reader.peek() == '#';
    if !tags && reader.can_read() && reader.peek() == '#' {
        return Err(CommandError::tr_at(
            "argument.block.tag.disallowed",
            &[],
            reader,
        ));
    }
    if tag {
        reader.skip();
    }
    let id = read_id(reader)?;
    let known = if tag {
        reg.block_tags.contains(&id) || reg.block_tags.is_empty()
    } else {
        reg.blocks.contains_key(&id) || reg.blocks.is_empty()
    };
    if !known {
        reader.cursor = start;
        let key = if tag {
            "arguments.block.tag.unknown"
        } else {
            "argument.block.id.invalid"
        };
        return Err(CommandError::tr_at(key, &[&id], reader));
    }
    let mut properties = Vec::new();
    if reader.can_read() && reader.peek() == '[' {
        reader.skip();
        reader.skip_whitespace();
        let allowed = reg.blocks.get(&id);
        while reader.can_read() && reader.peek() != ']' {
            reader.skip_whitespace();
            let at = reader.cursor;
            let name = reader.read_string()?;
            let property = allowed.and_then(|p| p.iter().find(|(n, _)| *n == name));
            if !tag && allowed.is_some() && property.is_none() {
                reader.cursor = at;
                return Err(CommandError::tr_at(
                    "argument.block.property.unknown",
                    &[&id, &name],
                    reader,
                ));
            }
            if properties
                .iter()
                .any(|(n, _): &(String, String)| *n == name)
            {
                reader.cursor = at;
                return Err(CommandError::tr_at(
                    "argument.block.property.duplicate",
                    &[&name, &id],
                    reader,
                ));
            }
            reader.skip_whitespace();
            if !reader.can_read() || reader.peek() != '=' {
                return Err(CommandError::tr_at(
                    "argument.block.property.novalue",
                    &[&name, &id],
                    reader,
                ));
            }
            reader.skip();
            reader.skip_whitespace();
            let vat = reader.cursor;
            let value = reader.read_string()?;
            if let Some((_, values)) = property
                && !values.contains(&value)
            {
                reader.cursor = vat;
                return Err(CommandError::tr_at(
                    "argument.block.property.invalid",
                    &[&id, &value, &name],
                    reader,
                ));
            }
            properties.push((name, value));
            reader.skip_whitespace();
            if reader.can_read() && reader.peek() == ',' {
                reader.skip();
                continue;
            }
            if !reader.can_read() || reader.peek() != ']' {
                return Err(CommandError::tr_at(
                    "argument.block.property.unclosed",
                    &[],
                    reader,
                ));
            }
        }
        if !reader.can_read() {
            return Err(CommandError::tr_at(
                "argument.block.property.unclosed",
                &[],
                reader,
            ));
        }
        reader.skip();
    }
    let nbt = if reader.can_read() && reader.peek() == '{' {
        Some(read_braced(reader, '{', '}')?)
    } else {
        None
    };
    Ok(BlockInput {
        id,
        properties,
        tag,
        nbt,
    })
}
/// Skip a balanced bracketed section (SNBT or component list) and return it.
fn read_braced(reader: &mut Reader, open: char, close: char) -> CmdResult<String> {
    let start = reader.cursor;
    let mut depth = 0;
    let mut quote: Option<char> = None;
    while reader.can_read() {
        let c = reader.read();
        match quote {
            Some(q) if c == '\\' => {
                let _ = q;
                reader.skip();
            }
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == open || c == '[' || c == '{' => depth += 1,
            None if c == close || c == ']' || c == '}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(reader.string[start..reader.cursor].to_owned());
                }
            }
            None => {}
        }
    }
    Err(CommandError::tr_at(
        "argument.nbt.expected.compound",
        &[],
        reader,
    ))
}
fn suggest_block(builder: &mut SuggestionsBuilder, reg: &Registries, tags: bool) {
    let remaining = builder.remaining().to_owned();
    if let Some(open) = remaining.find('[') {
        let id = normalize(&remaining[..open]);
        let Some(props) = reg.blocks.get(&id) else {
            return;
        };
        let inside = &remaining[open + 1..];
        if inside.contains(']') {
            return;
        }
        let part_start = inside.rfind(',').map_or(0, |i| i + 1);
        let part = &inside[part_start..];
        let base = builder.start + open + 1 + part_start;
        let used: Vec<&str> = inside
            .split(',')
            .filter_map(|p| p.split_once('=').map(|(n, _)| n.trim()))
            .collect();
        match part.split_once('=') {
            Some((name, value)) => {
                let mut b = builder.offset(base + name.len() + 1);
                if let Some((_, values)) = props.iter().find(|(n, _)| n == name.trim()) {
                    if values.iter().any(|v| v == value) {
                        let mut end = builder.offset(builder.input.len());
                        if used.len() < props.len() {
                            end.suggest(",");
                        }
                        end.suggest("]");
                        builder.out.extend(end.out);
                    } else {
                        b.suggest_matching(values.iter().map(String::as_str));
                    }
                }
                builder.out.extend(b.out);
            }
            None => {
                let mut b = builder.offset(base);
                for (n, _) in props {
                    if !used.contains(&n.as_str()) && n.starts_with(part.trim()) {
                        b.suggest(format!("{n}="));
                    }
                }
                if part.is_empty() {
                    b.suggest("]");
                }
                builder.out.extend(b.out);
            }
        }
        return;
    }
    let id = normalize(&remaining);
    if !remaining.is_empty() && reg.blocks.get(&id).is_some_and(|p| !p.is_empty()) {
        let mut end = builder.offset(builder.input.len());
        end.suggest("[");
        builder.out.extend(end.out);
    }
    let ids: Vec<String> = reg.blocks.keys().cloned().collect();
    suggest_resources(builder, &ids, "");
    if tags {
        suggest_resources(builder, &reg.block_tags, "#");
    }
}
fn parse_item(reader: &mut Reader, reg: &Registries, predicate: bool) -> CmdResult<ItemInput> {
    let start = reader.cursor;
    if predicate && reader.can_read() && reader.peek() == '*' {
        reader.skip();
        return Ok(ItemInput {
            id: "*".into(),
            tag: false,
            any: true,
            components: None,
        });
    }
    let tag = reader.can_read() && reader.peek() == '#';
    if tag && !predicate {
        return Err(CommandError::tr_at(
            "argument.item.tag.disallowed",
            &[],
            reader,
        ));
    }
    if tag {
        reader.skip();
    }
    let id = read_id(reader)?;
    let known = if tag {
        reg.item_tags.is_empty() || reg.item_tags.contains(&id)
    } else {
        reg.items.is_empty() || reg.items.contains(&id)
    };
    if !known {
        reader.cursor = start;
        let key = if tag {
            "arguments.item.tag.unknown"
        } else {
            "argument.item.id.invalid"
        };
        return Err(CommandError::tr_at(key, &[&id], reader));
    }
    let components = if reader.can_read() && reader.peek() == '[' {
        Some(read_braced(reader, '[', ']')?)
    } else {
        None
    };
    Ok(ItemInput {
        id,
        tag,
        any: false,
        components,
    })
}
/// TimeArgument units.
fn time_unit(unit: &str) -> Option<i32> {
    match unit {
        "" | "t" => Some(1),
        "s" => Some(20),
        "d" => Some(24000),
        _ => None,
    }
}

impl ArgType {
    pub fn parse(&self, reader: &mut Reader, reg: &Registries) -> CmdResult<Arg> {
        let start = reader.cursor;
        Ok(match self {
            ArgType::Bool => Arg::Bool(reader.read_boolean()?),
            ArgType::Integer { min, max } => {
                let v = reader.read_int()?;
                if v < *min {
                    reader.cursor = start;
                    return Err(CommandError::tr_at(
                        "argument.integer.low",
                        &[min, &v],
                        reader,
                    ));
                }
                if v > *max {
                    reader.cursor = start;
                    return Err(CommandError::tr_at(
                        "argument.integer.big",
                        &[max, &v],
                        reader,
                    ));
                }
                Arg::Int(v)
            }
            ArgType::Float { min, max } => {
                let v = reader.read_float()?;
                if v < *min {
                    reader.cursor = start;
                    return Err(CommandError::tr_at(
                        "argument.float.low",
                        &[min, &v],
                        reader,
                    ));
                }
                if v > *max {
                    reader.cursor = start;
                    return Err(CommandError::tr_at(
                        "argument.float.big",
                        &[max, &v],
                        reader,
                    ));
                }
                Arg::Float(v)
            }
            ArgType::Double { min, max } => {
                let v = reader.read_double()?;
                if v < *min {
                    reader.cursor = start;
                    return Err(CommandError::tr_at(
                        "argument.double.low",
                        &[min, &v],
                        reader,
                    ));
                }
                if v > *max {
                    reader.cursor = start;
                    return Err(CommandError::tr_at(
                        "argument.double.big",
                        &[max, &v],
                        reader,
                    ));
                }
                Arg::Double(v)
            }
            ArgType::Word => Arg::Str(reader.read_unquoted_string().to_owned()),
            ArgType::Str => Arg::Str(reader.read_string()?),
            ArgType::Greedy | ArgType::Message => {
                let s = reader.remaining().to_owned();
                reader.cursor = reader.string.len();
                Arg::Str(s)
            }
            ArgType::Entity { single, players } => {
                let s = parse_selector(reader, reg)?;
                if s.limit > 1 && *single {
                    reader.cursor = start;
                    let key = if *players {
                        "argument.player.toomany"
                    } else {
                        "argument.entity.toomany"
                    };
                    return Err(CommandError::tr_at(key, &[], reader));
                }
                if s.include_entities && *players && !s.is_self() {
                    reader.cursor = start;
                    return Err(CommandError::tr_at("argument.player.entities", &[], reader));
                }
                Arg::Entity(s)
            }
            ArgType::BlockPos => {
                if reader.can_read() && reader.peek() == '^' {
                    Arg::Coords(parse_local(reader)?)
                } else {
                    Arg::Coords(parse_world(reader, true, false)?)
                }
            }
            ArgType::Vec3 { center } => {
                if reader.can_read() && reader.peek() == '^' {
                    Arg::Coords(parse_local(reader)?)
                } else {
                    Arg::Coords(parse_world(reader, false, *center)?)
                }
            }
            ArgType::Rotation => {
                if !reader.can_read() {
                    return Err(CommandError::tr_at(
                        "argument.rotation.incomplete",
                        &[],
                        reader,
                    ));
                }
                let yaw = parse_coord_double(reader, false)?;
                if !reader.can_read() || reader.peek() != ' ' {
                    reader.cursor = start;
                    return Err(CommandError::tr_at(
                        "argument.rotation.incomplete",
                        &[],
                        reader,
                    ));
                }
                reader.skip();
                let pitch = parse_coord_double(reader, false)?;
                Arg::Coords(Coordinates::World([
                    yaw,
                    pitch,
                    Coord {
                        relative: true,
                        value: 0.0,
                    },
                ]))
            }
            ArgType::Angle => {
                if !reader.can_read() {
                    return Err(CommandError::tr_at(
                        "argument.angle.incomplete",
                        &[],
                        reader,
                    ));
                }
                let relative = reader.peek() == '~';
                if relative {
                    reader.skip();
                }
                let value = if reader.can_read() && reader.peek() != ' ' {
                    f64::from(reader.read_float()?)
                } else {
                    0.0
                };
                if value.is_nan() || value.is_infinite() {
                    return Err(CommandError::tr_at("argument.angle.invalid", &[], reader));
                }
                Arg::Angle(Coord { relative, value })
            }
            ArgType::Item => Arg::Item(parse_item(reader, reg, false)?),
            ArgType::ItemPredicate => Arg::Item(parse_item(reader, reg, true)?),
            ArgType::BlockState => Arg::Block(parse_block(reader, reg, false)?),
            ArgType::BlockPredicate => Arg::Block(parse_block(reader, reg, true)?),
            // ResourceOrTagKeyArgument: structures and biomes also take a #tag.
            ArgType::Resource(kind @ (Resource::Structure | Resource::Biome))
                if reader.can_read() && reader.peek() == '#' =>
            {
                reader.cursor += 1;
                Arg::Resource(format!("#{}", read_id(reader)?))
            }
            ArgType::Resource(kind) => {
                let id = read_id(reader)?;
                let values = kind.values(reg);
                if !values.is_empty() && !values.contains(&id) {
                    reader.cursor = start;
                    return Err(CommandError::new(crate::text::Text::tr(
                        "argument.resource.not_found",
                        &[&id, &kind.registry()],
                    )));
                }
                Arg::Resource(id)
            }
            ArgType::Id(_) => Arg::Resource(read_id(reader)?),
            ArgType::Dimension => {
                let id = read_id(reader)?;
                if !reg.dimensions.is_empty() && !reg.dimensions.contains(&id) {
                    return Err(CommandError::tr("argument.dimension.invalid", &[&id]));
                }
                Arg::Resource(id)
            }
            ArgType::Time { min } => {
                let v = reader.read_float()?;
                let unit = reader.read_unquoted_string();
                let Some(m) = time_unit(unit) else {
                    return Err(CommandError::tr("argument.time.invalid_unit", &[]));
                };
                let ticks = (v * m as f32).round() as i32;
                if ticks < *min {
                    return Err(CommandError::tr(
                        "argument.time.tick_count_too_low",
                        &[min, &ticks],
                    ));
                }
                Arg::Int(ticks)
            }
            ArgType::GameMode => {
                let name = reader.read_unquoted_string();
                match GameMode::from_name(name) {
                    Some(m) => Arg::GameMode(m),
                    None => {
                        reader.cursor = start;
                        return Err(CommandError::tr_at(
                            "argument.gamemode.invalid",
                            &[&name],
                            reader,
                        ));
                    }
                }
            }
            ArgType::Nbt => {
                if !reader.can_read() || reader.peek() != '{' {
                    return Err(CommandError::tr_at(
                        "argument.nbt.expected.compound",
                        &[],
                        reader,
                    ));
                }
                Arg::Str(read_braced(reader, '{', '}')?)
            }
            ArgType::Component => {
                let (value, used) =
                    crate::component::parse_snbt(reader.remaining()).map_err(|e| {
                        CommandError::tr_at("argument.component.invalid", &[&e], reader)
                    })?;
                reader.cursor += used;
                Arg::Str(value.to_string())
            }
            ArgType::Swizzle => {
                let mut axes = String::new();
                while reader.can_read() && reader.peek() != ' ' {
                    let c = reader.read();
                    if !matches!(c, 'x' | 'y' | 'z') || axes.contains(c) {
                        reader.cursor = start;
                        return Err(CommandError::tr_at(
                            "arguments.swizzle.invalid",
                            &[],
                            reader,
                        ));
                    }
                    axes.push(c);
                }
                Arg::Str(axes)
            }
            ArgType::Anchor => {
                let name = reader.read_unquoted_string();
                match name {
                    "eyes" => Arg::Anchor(true),
                    "feet" => Arg::Anchor(false),
                    _ => {
                        reader.cursor = start;
                        return Err(CommandError::tr_at(
                            "argument.anchor.invalid",
                            &[&name],
                            reader,
                        ));
                    }
                }
            }
        })
    }
    pub fn suggest(
        &self,
        builder: &mut SuggestionsBuilder,
        reg: &Registries,
        context: &Context,
        _name: &str,
    ) {
        let _ = context;
        match self {
            ArgType::Bool => builder.suggest_matching(["true", "false"]),
            ArgType::Entity { .. } => suggest_selector(builder, reg),
            ArgType::BlockPos => suggest_coordinates(builder, reg),
            ArgType::Vec3 { .. } => suggest_coordinates(builder, reg),
            ArgType::Rotation => {
                if builder.remaining().is_empty() {
                    builder.suggest("~");
                    builder.suggest("~ ~");
                }
            }
            ArgType::Angle => {
                if builder.remaining().is_empty() {
                    builder.suggest("~");
                }
            }
            ArgType::Item | ArgType::ItemPredicate => {
                let items = reg.items.clone();
                suggest_resources(builder, &items, "");
                if matches!(self, ArgType::ItemPredicate) {
                    suggest_resources(builder, &reg.item_tags, "#");
                }
            }
            ArgType::BlockState => suggest_block(builder, reg, false),
            ArgType::BlockPredicate => suggest_block(builder, reg, true),
            ArgType::Resource(kind) => {
                let values = kind.values(reg).to_vec();
                suggest_resources(builder, &values, "");
            }
            ArgType::Id(list) => {
                let values = match *list {
                    "time_markers" => reg.time_markers.clone(),
                    "advancements" => reg.advancements.clone(),
                    _ => Vec::new(),
                };
                suggest_resources(builder, &values, "");
            }
            ArgType::Dimension => {
                let dims = reg.dimensions.clone();
                suggest_resources(builder, &dims, "");
            }
            ArgType::Time { .. } => {
                let mut r = Reader::new(builder.remaining());
                if r.read_float().is_ok() {
                    let mut b = builder.offset(builder.start + r.cursor);
                    b.suggest_matching(["d", "s", "t"]);
                    builder.out.extend(b.out);
                }
            }
            ArgType::GameMode => builder.suggest_matching(GameMode::ALL.map(GameMode::name)),
            ArgType::Anchor => builder.suggest_matching(["eyes", "feet"]),
            ArgType::Swizzle | ArgType::Component => {}
            ArgType::Message => {
                // MessageArgument suggests selectors and names for the last word.
                let rem = builder.remaining();
                let word_start = rem.rfind(' ').map_or(0, |i| i + 1);
                let mut b = builder.offset(builder.start + word_start);
                if b.remaining().starts_with('@') || !b.remaining().is_empty() {
                    let players = reg.players.clone();
                    b.suggest_matching(players.iter().map(String::as_str));
                    if b.remaining().starts_with('@') {
                        b.suggest_matching(SELECTORS.map(|s| s.0));
                    }
                }
                builder.out.extend(b.out);
            }
            _ => {}
        }
    }
}
