//! Parsed native command input and the source that executes it.

use crate::{DimensionKey, EntityRef, Identifier, ItemStack, LevelRef, PermissionLevel, PlayerRef};
use horizon_core::nbt::Compound;

/// The origin of a registered mod command, independent of `execute as`'s executor.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum CommandSource {
    /// A player's command; their state is swapped in for the listener.
    Player {
        /// The player who submitted the command.
        player: PlayerRef,
        /// The execution permission, including function overrides.
        permission: PermissionLevel,
    },
    /// The dedicated server console or RCON.
    Console {
        /// The execution permission.
        permission: PermissionLevel,
    },
    /// A command block, including when `execute` moves its execution position.
    CommandBlock {
        /// The block's position, not the execution position.
        position: [i32; 3],
        /// The block's dimension.
        level: DimensionKey,
        /// The execution permission.
        permission: PermissionLevel,
    },
    /// A command-block minecart.
    CommandBlockMinecart {
        /// The minecart that submitted the command.
        entity: EntityRef,
        /// The execution permission.
        permission: PermissionLevel,
    },
    /// The innermost executing function; feedback still follows its caller.
    Function {
        /// The function's namespaced identifier.
        id: Identifier,
        /// The function's execution permission.
        permission: PermissionLevel,
    },
}

impl CommandSource {
    /// The effective permission checked before parsing and argument resolution.
    pub fn permission(&self) -> PermissionLevel {
        match self {
            Self::Player { permission, .. }
            | Self::Console { permission }
            | Self::CommandBlock { permission, .. }
            | Self::CommandBlockMinecart { permission, .. }
            | Self::Function { permission, .. } => *permission,
        }
    }
}

/// One validated native command argument, resolved against its command source.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum CommandArgumentValue {
    /// A `word` or greedy `text` argument, exactly as the dispatcher parsed it.
    String(String),
    /// A signed `integer` argument.
    Integer(i32),
    /// A finite double-precision `number` argument.
    Number(f64),
    /// A `boolean` argument.
    Boolean(bool),
    /// The single online player selected by a `player` argument. Side: server.
    Player(PlayerRef),
    /// The single handled entity selected by an `entity` argument. Side: server.
    /// Unresolvable handles are refused, not delivered.
    Entity(EntityRef),
    /// Handled entities selected by `entities`, in selector order. Side: server.
    ///
    /// Entities without a resolvable handle are omitted; an empty result refuses the
    /// command. Lists have no additional cap beyond the wire frame limits.
    /// Dragon parts are not selector members. Leash knots, item frames,
    /// paintings and tridents are delivered when their residency handle resolves;
    /// temporarily held-out and removed entities are omitted.
    Entities(Vec<EntityRef>),
    /// Online players selected by `players`, in selector order. Side: server.
    /// Nonempty, with no additional cap beyond the wire frame limits.
    Players(Vec<PlayerRef>),
    /// An `item` argument with its components applied and a count of one.
    Item(ItemStack),
    /// A `block_pos` argument resolved from world, relative or local coordinates.
    /// Resolution does not load chunks or require the block to be loaded.
    BlockPos([i32; 3]),
    /// A `vec3` or `vec3_uncentered` argument resolved against the source: `~` and `^` offsets
    /// applied to its position and rotation. `vec3` centres whole `x` and `z` on their block
    /// (`5` is `5.5`); `vec3_uncentered` keeps them as written. Resolution does not load chunks.
    Vec3([f64; 3]),
    /// A `column_pos` argument: a block `x z`, `~` resolved against the source's block and
    /// floored.
    ColumnPos([i32; 2]),
    /// A `rotation` argument as yaw and pitch in degrees, `~` resolved against the source's
    /// rotation, as `entity_rotation` returns them.
    Rotation([f32; 2]),
    /// A `dimension` argument: the level of a dimension the world has loaded. An unknown one is
    /// refused with `argument.dimension.invalid`. Side: server.
    Dimension(LevelRef),
    /// An `nbt` argument: a compound tag written as SNBT.
    Nbt(Compound),
    /// A custom argument type's canonical value: the registered type's id
    /// (`mymod:machine`) and what its parser returned for the typed text.
    Custom {
        /// The id of the registered `CommandArgumentType`.
        kind: String,
        /// The value `CommandArgumentType::parse` returned.
        value: String,
    },
    /// A `resource:<registry>` argument: an id present in that registry (static or data pack).
    Id(Identifier),
}

/// An argument's registered name and its validated value.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct CommandArgument {
    /// The name from the command's usage, without its kind or brackets.
    pub name: String,
    /// The value produced by parsing and source resolution.
    pub value: CommandArgumentValue,
}

impl CommandArgument {
    /// Pair a registered argument name with its parsed value.
    pub fn new(name: String, value: CommandArgumentValue) -> Self {
        Self { name, value }
    }
}

/// A native command parsed once by the server's dispatcher, never a raw line.
///
/// Missing optional arguments are absent. Selectors are permission-checked and resolved
/// on the server before any listener runs. `entity` and `player` select exactly one;
/// `entities` and `players` select one or more, in selector order. Unhandled entities
/// are omitted, and an empty result is refused. Failed parsing or resolution produces
/// command feedback instead. Every source receives this same data on native and wasm.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct CommandInvocation {
    /// The literal path, including the registered command name and subcommands.
    pub literals: Vec<String>,
    /// Present arguments in parse order; omitted optional arguments have no entry.
    pub arguments: Vec<CommandArgument>,
    /// The level the command executes in, after `execute in`; the console's is the overworld.
    pub dimension: DimensionKey,
    /// The execution position, after `execute at` or `positioned`; the console's is the
    /// world spawn's block corner.
    pub position: [f64; 3],
}

impl CommandInvocation {
    /// Build an invocation from its parsed literal path, resolved arguments and execution
    /// level and position.
    pub fn new(
        literals: Vec<String>,
        arguments: Vec<CommandArgument>,
        dimension: DimensionKey,
        position: [f64; 3],
    ) -> Self {
        Self {
            literals,
            arguments,
            dimension,
            position,
        }
    }

    /// The last argument registered under `name`, or `None` when it is absent.
    pub fn argument(&self, name: &str) -> Option<&CommandArgumentValue> {
        self.arguments
            .iter()
            .rev()
            .find(|argument| argument.name == name)
            .map(|argument| &argument.value)
    }
}

/// A command's outcome and the messages received by its source.
///
/// The default is an unsuccessful run with no feedback, including when a handle
/// cannot resolve or its host has no backend.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct CommandResult {
    /// Whether the command parsed and ran successfully, as `EntityRef::run_command` reports.
    pub success: bool,
    /// Success and failure messages received by the source, in order, as plain text.
    /// Player chat delivery is preserved. Silent sources have no feedback lines.
    pub feedback: Vec<String>,
}

impl CommandResult {
    /// Build a result from the execution outcome and the source's feedback lines.
    pub fn new(success: bool, feedback: Vec<String>) -> Self {
        Self { success, feedback }
    }
}

pub(crate) mod command_argument;
pub(crate) mod entity_selector;
pub(crate) mod suggestion;
pub(crate) mod types;
