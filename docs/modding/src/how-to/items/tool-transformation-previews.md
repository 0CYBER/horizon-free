# Tool transformation previews

Use `LevelRef::block_transform` to preview a tool's effect on a loaded block in
the server's active level. Pass the live player, hand, stack, position and hit.
The call returns an owned `BlockTransformPreview` without applying it.

- The preview describes the resulting block state, relative durability cost,
  sound, loot and particle.
- The call fires `block_tool_modification` with `ToolUse::simulated` set to
  `true`. Your listener must not change the level during a simulation. The real
  action fires the same hook with `simulated` set to `false`.
- A stale or unavailable player or level, a player in another dimension, an
  inactive level, an unloaded position, an invalid face, or a tool with no
  transformation returns `None`.
