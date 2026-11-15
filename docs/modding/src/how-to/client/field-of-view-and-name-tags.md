# Field of view and name tags

Two client hooks change what the local player sees without drawing anything yourself.

## Field of view

`compute_fov_modifier` runs on the client once per tick while there is a local player, after vanilla's effects
(sprinting, a drawn bow) and before zoom and camera easing. It is NeoForge's `ComputeFovModifierEvent`.

| `ComputeFovModifierEvent` field | Meaning |
| --- | --- |
| `player` | The inputs of the calculation (`FovPlayer`) |
| `base` | The scale before any mod ran |
| `modifier` | The scale to use; write it to change the field of view |

A value that is not finite, or lies outside `0.01..=10`, falls back to `base`. The hook does not affect a spectator
camera attached to a mob.

## Name tags

`render_name_tag` runs when the game decides whether to show the name tag of a player, mob, armor stand or item
frame, before the tag is drawn. It is NeoForge's `RenderNameTagEvent`.

| `RenderNameTagEvent` field | Meaning |
| --- | --- |
| `entity_id`, `kind` | The entity and its type id |
| `vanilla_visible` | What vanilla decided |
| `visible` | Set it to show or hide the tag |
| `text` | Set it to `Some(text)` to replace the tag's text |

Distance and the entity's availability still apply. The local player's own tag is considered only when its camera
and settings would show it.

## The client session

For a HUD that shows session information, read `ClientHost::client_session()`, and the local `PlayerRef`'s
`block_breaking` and `ambient_mood`. Values that may be missing (a session, an external display name) are
`Option`s, and every counter stays on the client. The [hook notes](../../reference/hook-notes.md#client-session-and-local-player-status)
give their exact availability and timing.
