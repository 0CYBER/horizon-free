# Horizon extras

Optional conveniences built into the client: zoom, a minimap, an FPS counter, a
keystroke overlay and auto-reconnect. **Every one is off by default**: a fresh
install looks and behaves exactly like vanilla 26.3, and turning an extra off
again restores vanilla's behaviour completely.

## Where they live

| Part | Code |
|---|---|
| Settings, parsing and saving | `crates/horizon-client/src/extras.rs` |
| Zoom curve, FPS and click counters | `extras.rs` (pure logic, unit tested) |
| HUD drawing (FPS, keystrokes, minimap frame) | `crates/horizon-client/src/extras_hud.rs` |
| Minimap colours, tiles and image | `crates/horizon-client/src/minimap.rs` |
| Reconnect policy and backoff | `crates/horizon-client/src/reconnect.rs` |
| Game glue (input hooks, per-frame update) | `crates/horizon-client/src/game/extras.rs` |
| The settings page | `options_horizon.rs`: the **Extras** section of Horizon's Options |

`game.rs` only gets one-line hooks (key, wheel, mouse, frame, HUD), so the
extras stay out of the way of other input and options work.

## Options storage

`horizon-extras.txt` beside `options.txt`, in the same `key:value` format as
`horizon-graphics.txt`. `options.txt` is untouched, so a vanilla client can share
the folder. Unknown or broken lines fall back to the defaults.

The zoom key is a Horizon setting stored in that file (`zoomKey:key.keyboard.z`)
and bound on the Extras page, not in vanilla's Key Binds screen: Horizon has no
Horizon category in Controls, and adding one would put a non-vanilla mapping into
`options.txt`.

## The extras

### Zoom (`zoom`, default off)
- Hold the zoom key (default **Z**, unused by every vanilla 26.3 key mapping; C
  is `key.saveToolbarActivator`) to zoom. The key does nothing while zoom is off.
- It zooms only while playing: in chat, signs, books, text fields and every
  other screen the key is typed or handled as usual. The key is never taken
  from vanilla's mappings: one bound to the same key still acts (a key bound
  twice triggers both, as vanilla's do), and the Extras page marks the zoom key
  "Also bound" and names the mapping.
- Starts at the default level (4x, settable 2x to 10x); the scroll wheel changes
  it while the key is held (and does not move the hotbar then).
- **Smooth** (default on) goes through vanilla's own FOV easing: the zoom divides
  the field-of-view modifier target `Camera.tickFov` eases toward, the path the
  spyglass uses (`AbstractClientPlayer.getFieldOfViewModifier` returns 0.1 while
  scoping). The modifier keeps vanilla's 0.1 floor, so 10x is the deepest zoom.
  Off applies the zoom to the final FOV with no easing.
- While scoping with a spyglass the zoom does nothing: the spyglass view and its
  overlay stay exactly vanilla's.
- **Lower sensitivity** (default on) divides mouse turning by the zoom factor.
- With zoom off (or the key up) the FOV target and mouse are vanilla's, bit for
  bit; the FOV harness covers this.

### FPS counter (`fps`, default off)
- A small "N fps" line in a corner (default top left), optionally followed by
  the minimum and the 1% low over the last 5 seconds.
- Separate from the Advanced graphics "Overlay" (the frame-time graph).

### Keystrokes (`keystrokes`, default off)
- W A S D, Space, LMB and RMB keys that light up while held, with clicks per
  second under the mouse buttons (clicks in the last second). The letters follow
  the bound keys.

### Minimap (`minimap`, default off)
- A corner map (top right by default), square or round, small/medium/large,
  zoom 0.5 to 4 blocks per pixel, rotating with the player or north up, with the
  player arrow (vanilla's map decoration sprite).
- Colours are vanilla's `MapColor` per block, as map items use, with the map
  item's height shading and water-depth dithering.
- Underground, in the Nether and under any roof, it maps the floor around the
  player's height instead of the top block, so caves read sensibly.
- Cost: one cached 16x16 tile per loaded chunk (packed map colour and height).
  Tiles are filled nearest first and refreshed when blocks change, within a time
  budget of 0.3 ms a frame; composing the 128x128 image is a few thousand
  lookups, done only when the view moved or a tile changed, and uploaded into a
  dedicated atlas slot. With the minimap off nothing runs.

### Auto-reconnect (`autoReconnect`, default off)
- On the "Connection lost" screen after a multiplayer connection drops, a
  countdown "Reconnecting in N s" with a Cancel button, then a new connection
  to the same address.
- Backoff 5, 10, 20, 40, 60 s; at most 5 attempts, then it stops. The count
  only resets after staying connected for a minute, so a server that drops
  players at once never causes an endless loop.
- Never after a kick, a ban, a whitelist or version refusal, a duplicate login,
  an online-mode refusal (no account signed in), or a session refusal from the
  account's login (invalid session, banned from playing online, multiplayer
  disabled, the server failing to verify the username). Unreachable session
  servers, on either side, are retried with the same backoff.
- Online-mode servers: the retry joins with the account the game signed in
  with, held in memory for the whole run (`auth::current()`); nothing is
  written to disk.

## Shared rules

- All extras are hidden with F1 and drawn in the HUD layer: under screens, over
  the world, like vanilla's HUD. Extras in the top left corner give way to the
  debug screen (F3), which shows the FPS anyway.
- Corners stack: extras placed in the same corner stack downward (top corners)
  or upward (bottom corners) in the order minimap, FPS, keystrokes. The top
  right corner starts below the status effect icons.

## Perf budget

Off: a flag check per frame per extra (no allocations, no world access).
On: FPS and keystrokes are a few quads; the minimap stays under 0.3 ms of CPU
per frame (tile budget) plus a 64 KiB texture upload when its image changes.
