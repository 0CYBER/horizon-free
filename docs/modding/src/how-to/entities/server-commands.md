# Typed server commands

Register each server usage with `register_commands` and `Command`. The
`run_mod_command` event carries a `CommandInvocation`,
not a raw line to split. Match its `literals` to your command path, then read named
values through `command.argument(name)` and `CommandArgumentValue`. An integer is
already an `i32`; a player argument is a live `PlayerRef`, not selector text.
Return `Flow::Cancel` when your listener handles the command.

The server parses and checks permissions before resolving arguments or calling a
listener. Omitted optional arguments are absent, including an omitted optional
player: they do not silently select the source player. `player` selects exactly one
online player, `players` one or more online players, `entity` exactly one entity,
and `entities` one or more entities. Missing or too-large selections produce
vanilla feedback without calling the listener. Word and greedy text arguments
are strings, numbers are finite doubles, booleans are typed, and item arguments
are component-bearing stacks of one item. `mod_id` takes a loaded package id and
delivers it as a string (NeoForge's `ModIdArgument`). `resource:<registry>`, such as
`<kind:resource:minecraft:entity_type>`, takes an id that registry holds, static or
from a data pack, and delivers `CommandArgumentValue::Id`; any other id is refused
with vanilla's `argument.resource.not_found` before a listener runs. Clients see it
as vanilla's `minecraft:resource_key`, so unmodded clients complete it too. The registry
must be a `namespace:path` resource location of at most 256 bytes, and the server takes at
most 256 distinct registries; a usage past either is left out, with the reason in the log. Block positions are resolved and floored
from the source's world, relative or local coordinates without loading chunks.

Positions, rotations, dimensions and compound tags have their own kinds, each sent to clients
as vanilla's parser so `~`, `^` and the dimension list complete as in a vanilla command:

| Kind | Vanilla parser | `CommandArgumentValue` |
|---|---|---|
| `vec3` | `minecraft:vec3`, whole `x` and `z` centred on the block | `Vec3([f64; 3])` |
| `vec3_uncentered` | `minecraft:vec3`, coordinates as written | `Vec3([f64; 3])` |
| `column_pos` | `minecraft:column_pos` | `ColumnPos([i32; 2])`, floored |
| `rotation` | `minecraft:rotation` | `Rotation([f32; 2])`, yaw then pitch |
| `dimension` | `minecraft:dimension` | `Dimension(LevelRef)` |
| `nbt` | `minecraft:nbt_compound_tag` | `Nbt(Compound)` |

`~` and `^` resolve against the command source's position and rotation, as `block_pos` does. A
`dimension` the world has not loaded is refused with `argument.dimension.invalid` before a
listener runs. A `vec3` and a `vec3_uncentered` cannot share a position in one command's
usages, as they look the same to a client.
A player's source is their position and rotation. The console has no source
entity and uses the Overworld spawn's block corner with zero rotation, so `@s`
cannot resolve there. Every source receives the same owned data, on native and wasm.

`event.source` is a non-exhaustive `CommandSource`: `Player { player, .. }`,
`Console { .. }`, `CommandBlock { position, level, .. }`,
`CommandBlockMinecart { entity, .. }` or `Function { id, .. }`.
`source.permission()` returns the effective `PermissionLevel`: functions use
function permission; blocks and carts use game-master permission. `execute as`,
`at` and `in` change argument resolution, not the origin. `command.dimension` and
`command.position` are where the command executes after `execute in`, `at` or
`positioned`; the console executes in the Overworld at its spawn corner.

Two reads work in a command and elsewhere on the server. `player.profile(host)` returns
the player's `GameProfile`: UUID, name and login properties such as the signed `textures`
skin (empty for an offline-mode player). `host.advancement(&id)` returns a loaded
`AdvancementInfo`: its parent, display (title, description, icon stack with its components, frame and
flags), criterion names and requirements, from the set the last `/reload` built. Feedback through
`host.commands().feedback` honours the underlying source's silence and track-output.

## Run a command as a player

`player.entity().run_command_with_output(host, line, level)` runs a command as that
player, from their position and rotation, with `@s` selecting them. Pass
`PermissionLevel::from_level(i32::from(player.permission_level(host)?))` to use
the player's own permission level. The supplied level is taken as given, so a mod
can also choose a different level explicitly. A leading slash is allowed.

The returned `CommandResult` has `success` (the same bool as `run_command`) and
`feedback`: the success and failure lines received by the source, in order, as
plain text. The player still receives those lines in chat, and operator
notifications follow the normal rules. Feedback silenced by the source or game
rules is absent. Other entities use a silent source and return no lines; a stale,
removed or unavailable entity returns `success: false` and no lines. Teleports to
another level still happen at the next tick's start. Native and wasm mods use the
same method and runner. `run_command(host, line, level)` keeps its bool return.

## Entity and player selectors

Side: server. Register `Command::new("heal <targets:players>")` to receive
`CommandArgumentValue::Players(Vec<PlayerRef>)`: `/heal @a` calls the listener once
with all selected online players, not once per player. The other selector kinds
deliver `Player(PlayerRef)`, `Entity(EntityRef)` and `Entities(Vec<EntityRef>)`.
For example, `<target:entity>` accepts
`@e[type=minecraft:cow,limit=1,sort=nearest]`. The shared vanilla selector parser
and resolver use the command source's position and selector permission, preserving
selection order (including `sort=nearest`, `sort=furthest` and `sort=random`).
The client's command tree carries the matching single/multiple and players-only
flags and offers the normal player-name and selector completions.

Empty selections report `argument.entity.notfound.player` or
`argument.entity.notfound.entity`; multiple-result selectors passed to a single
argument report `argument.player.toomany` or `argument.entity.toomany`.
A source without selector permission receives
`argument.entity.selector.not_allowed` before resolution. Plain online player
names do not require selector permission.

In a multi-entity selection, an entity that cannot resolve to an `EntityRef` is
left out. If none remain, or a single target cannot resolve, the command reports `argument.entity.notfound.entity`. There is no
entity-type exclusion: leash knots, item frames, paintings and tridents
are delivered when resolvable. Dragon parts
are not selector members in vanilla and are never delivered by these arguments.
Temporarily held-out or removed entities cannot be delivered. An unbounded
selector can reach other loaded dimensions; handle lookup uses the entity's own
dimension. Lists are bounded only by the wire frame limits, on native and wasm;
there is no extra count cap. A very large `@e` is the caller's cost.

## Custom argument types

Register a value kind once from `Hooks::register`, then use its namespaced id as an
argument's kind:

```rust,ignore
use mod_api::{
    Command, CommandArgumentError, CommandArgumentKind, CommandArgumentType, CommandSuggestion,
    Hooks, RegisterCommandsEvent, Registrar, RegistrationContext, ServerHost, Text,
};

struct Machine;

impl CommandArgumentType for Machine {
    fn kind(&self) -> CommandArgumentKind {
        CommandArgumentKind::Word
    }

    fn parse(&self, input: &str) -> Result<String, CommandArgumentError> {
        match input {
            "press" | "furnace" => Ok(input.to_owned()),
            _ => Err(CommandArgumentError::new(Text::translate(
                "example.commands.machine.unknown",
                vec![Text::literal(input)],
            ))),
        }
    }

    fn suggest(&self, prefix: &str) -> Vec<CommandSuggestion> {
        ["press", "furnace"]
            .into_iter()
            .filter(|machine| machine.starts_with(prefix))
            .map(CommandSuggestion::new)
            .collect()
    }
}

#[mod_api::horizon_mod]
impl Hooks for MyMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.register_command_argument("machine", Machine);
    }

    fn register_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        event
            .commands
            .push(Command::new("machine use <machine:mymod:machine>"));
    }
}
```

The server parses the argument with `parse` before calling `run_mod_command`; a
rejected value shows the returned `Text` and no listener runs. A valid value arrives
as `CommandArgumentValue::Custom { kind, value }`, where `kind` is `mymod:machine`
and `value` is the canonical text `parse` returned. `suggest` is asked for the
completions while the player types.

For a fixed set of words, register `ChoiceArgument` (NeoForge's `EnumArgument`)
instead of writing a type: `r.register_command_argument("mode",
ChoiceArgument::new(["low", "high"]))`. It refuses other words, listing the valid
ones, and completes the choices that start with the typed text.

`CommandArgumentKind::Word` takes one word of ASCII letters, digits and `_ - . +`; the
word ends at the first other character, a space included, so a value with `:` or `/`
needs `Greedy`, which takes the rest of the line and must be the usage's last argument.
Players whose client does not know the type
parse and complete the command: the server sends the argument as a Brigadier string
of that shape with `minecraft:ask_server`, and their chat asks the server for the
suggestions.

The namespaced id is in the registering mod's namespace; an id registered twice, a
name in another namespace or an empty name refuses the mods at startup, naming the
mod. A client mod registers client command argument types the same way; they are
available to `register_client_commands`.

`player_command` provides player-only pre-execution cancellation (NeoForge's `CommandEvent`), including vanilla commands.

When the usages a mod registers change at runtime, call
`host.commands().refresh_commands()` on the server. At the start of the next
server tick, every mod is asked to register again. The server rebuilds the typed
command dispatcher and sends each player's tree after `player_commands_sent`
filters it, as vanilla does after `/reload`. Repeated requests before that tick
coalesce; without a world open to players, nothing is rebuilt.

## Custom selector tokens

Side: server. Register a token such as `@allies` once from `Hooks::register` with
`r.register_entity_selector("allies", Allies)`, where `Allies` implements
`EntitySelectorType` (NeoForge's `EntitySelectorManager.register` and
`IEntitySelectorType`). `base` returns its `SelectorBase`: players only, a limit, a
`SelectorSort` and the translation key of its completion tooltip. `matches(host, source,
entity)` is asked for each candidate after the base selection and the `[...]` options,
before sorting and the limit. Return `false` to drop the candidate; a panic drops it too.

Every entity, entities, player and players argument then accepts `@allies[...]`, in
vanilla commands and mod commands alike, with the same options and selector permission
as `@e`. The parser reads the whole word after `@` before vanilla's letter, so `@allies`
is never `@a`, and an unregistered word still fails with
`argument.entity.selector.unknown`. A name that is empty, not lowercase letters, digits
and `_`, one of `p a r s e n`, or already registered by any mod refuses the mods at
startup. The base is evaluated once at registration; a panic or zero limit refuses
registration. Token names and tooltip keys must fit the protocol string boundary
(32767 ASCII bytes for names, 32767 UTF-16 units for tooltip keys). Horizon also
bounds the complete encoded metadata to 256 KiB and refuses the token that would exceed
that total by name, without changing earlier records. The payload's revision is 1; an
unsupported revision disables token metadata for that connection with one log line.

The server sends frozen token metadata alongside each command tree to connected Horizon
clients, including the single-player window. Their command parser and completion use the
server's names, tooltip keys, base limits and sort order, even without the server's mod
installed locally. This replaces the previous server's inventory at connection startup.
Vanilla clients do not receive this Horizon-specific metadata and can still send commands
containing the token without local completion.
