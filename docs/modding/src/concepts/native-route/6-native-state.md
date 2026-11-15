# Native state: `[[native.state]]` is refused

Horizon refuses `[[native.state]]` at native build time: adding a field changes a game type's layout,
which must match the prebuilt game. If you declare this table, the build reports:

```text
E_MIXIN_STATE_LAYOUT: <mod>:<row>: native state changes the prebuilt game's layout; use an attachment instead
```

Remove the row and use an **attachment**. An attachment is a typed value the game keeps for you on an
entity, block entity, chunk, level or player, and saves with it when you give it a codec. It works the same
on native and wasm. See [server attachments](../attachments.md) and
[querying attachment holders](../attachments/querying.md).

For data about the whole game, a [published loop](3-loops.md)'s slot value lives once per `Game`. A `static` in
your crate is shared by every `Game` in the process and outlives each world: key it by world and clear it yourself.
