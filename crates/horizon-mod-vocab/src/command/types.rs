//! Commands and their authority.

/// Literal words use lowercase letters, digits, `_` and `-`. Only the first word may
/// additionally begin with one `/`: register `/set <x:word>` and type `//set stone` in chat.
/// The command's literals retain `/set`. A slash alone, repeated leading slashes and
/// slashes in later literal words are refused.
///
/// A command a native mod adds, as one usage: words, then arguments
/// `<name:kind>`, optional ones last in brackets `[name:kind]`:
/// `pay <player:player> <amount:integer>`, `balance [player:player]`,
/// `eco give <player:player> <amount:integer>`. Kinds: `word`, `text` (the
/// rest of the line, last), `integer`, `number`, `boolean`, `player`, `players`,
/// `entity`, `entities`, `item`, `block_pos`, `vec3` (a point, `Vec3Argument.vec3()`; delivered
/// as `Vec3`), `id` (a free `namespace:path` identifier, delivered as that canonical string),
/// `mod_id` (a loaded package id, delivered as a
/// string), `resource:<registry>` (an id that registry holds, delivered as `Id`:
/// `<kind:resource:minecraft:entity_type>`), or a command argument type a mod registered with
/// `Registrar::register_command_argument`, named by its namespaced id:
/// `machine use <machine:mymod:machine>`. Usages of one command share their
/// words; the server merges them into one tree. Each executable usage keeps its own level:
/// a public `foo bar` lets everyone traverse `foo`, but does not make an
/// operator-only `foo` executable. Optional arguments also allow execution at
/// their preceding prefixes, at that usage's level.
///
/// Side: server. Selector arguments use vanilla's source position, permissions,
/// cardinality and selection order; `player`/`entity` require one result and
/// `players`/`entities` require a nonempty result. Unresolvable entity handles are
/// omitted; lists have no extra cap beyond the wire frame limits.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct Command {
    /// The usage.
    pub usage: String,
    /// The operator level a player needs to run this usage (0: everyone).
    /// Shared prefixes stay visible when needed to reach a lower-level usage.
    pub level: u8,
}

impl Command {
    /// A usage every player may run.
    pub fn new(usage: &str) -> Self {
        Self {
            usage: usage.to_owned(),
            level: 0,
        }
    }
    /// The same, for operators of `level` and above.
    pub fn op(usage: &str, level: u8) -> Self {
        Self {
            usage: usage.to_owned(),
            level: level.min(4),
        }
    }
}

/// Which `Game` observes the phase; a mirror `Game` never fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum Authority {
    /// A single-player game with no server thread; does not fire.
    LocalAuthoritative,
    /// The server thread of a single-player or LAN world.
    IntegratedServer,
    /// A dedicated server.
    DedicatedServer,
    /// A client of someone else's server; does not fire.
    RemoteClient,
}

/// `PermissionLevel`: how much a player may do, as `op-permission-level` and
/// the operator list name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PermissionLevel {
    /// 0: every player.
    All,
    /// 1: moderators.
    Moderators,
    /// 2: game masters.
    Gamemasters,
    /// 3: admins.
    Admins,
    /// 4: owners.
    Owners,
}

impl PermissionLevel {
    /// The level a list entry's number names; out of range keeps to 0 or 4.
    pub fn from_level(level: i32) -> Self {
        match level {
            i32::MIN..=0 => Self::All,
            1 => Self::Moderators,
            2 => Self::Gamemasters,
            3 => Self::Admins,
            _ => Self::Owners,
        }
    }
    /// The level as the list stores it, 0 to 4.
    pub fn level(self) -> u8 {
        self as u8
    }
}
