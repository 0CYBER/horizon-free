# Playing level events

A level event is vanilla's numbered effect at a block: a door's sound, a block's break particles, a
dispenser's smoke. Vanilla has two methods for it, and a `LevelRef` has both. Both are server operations
on the level in place.

- `LevelRef::level_event(host, id, pos, data)` is `Level.levelEvent(null, id, pos, data)`: the players of
  that level within 64 blocks of `pos` get it.
- `LevelRef::global_level_event(host, id, pos, data)` is `ServerLevel.globalLevelEvent`: every player on
  the server gets it, in every level and at any distance.

Both return `false` without playing anything for an event that carries a block state (2001, 2014, 3008)
when `data` is not one, for a parked level, for a stale handle and on a client.

## The global events

Vanilla plays three ids through the global form: 1023, the wither's spawn (`WitherBoss`); 1028, the ender
dragon's death (`EnderDragon`); 1038, an End portal opening when the last eye of ender goes into its frame
(`EnderEyeItem.useOn`). The client has a sound for them only in its global handler
(`LevelEventHandler.globalLevelEvent`), so use `global_level_event` for them: as plain level events they
play nothing.

The client plays the sound two blocks from the camera, towards the position its packet carries. Each
player gets a position of their own, as vanilla's `globalLevelEvent` sends it:

| The player | The position they get |
| --- | --- |
| In the event's level, less than 32 blocks from the centre of `pos` | `pos` |
| In the event's level, 32 blocks or more away | the block 32 blocks from them towards the centre of `pos` |
| In another level | the block they stand in |

"The event's level" is the level the `LevelRef` names, the one the event happened in, whatever level
the server is ticking when the packets go out. Distances are from the player's feet. So a player in the
event's level hears it from the event's direction, and a player in another level hears it close by.

With the `global_sound_events` game rule off, `global_level_event` is a plain `level_event`, as in
vanilla: only the players of that level within 64 blocks of `pos` get it, and 1023, 1028 and 1038 play
nothing.
