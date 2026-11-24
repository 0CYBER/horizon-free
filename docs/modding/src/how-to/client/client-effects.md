# Client effects and tooltips

Use client services to edit tooltips, draw extra camera views and change local sound output. See the [API reference](../../reference/api.md) for the generated signatures.

## Reading the tooltip preference

Read `ClientHost::gui_options().advanced_item_tooltips` for the current
`advancedItemTooltips` preference (F3+H). The snapshot reflects the current client
setting on native and wasm; request a new snapshot after a toggle. This is separate
from the advanced flag supplied to an individual tooltip request.

## Adding tooltip lines

Use `ItemPart::tooltip` for an attached item's own lines at the vanilla
`Item.appendHoverText` position, after the name and before component lines.
Use `Hooks::item_tooltip` for NeoForge's global `ItemTooltipEvent` moment: it
receives every item's finished tooltip, including the parts' earlier lines,
component lines and advanced information. No attached part is needed for a
global listener, so this is also how a mod edits vanilla or other mods' tooltips.
The global event supplies the stack, display settings, player IDs, and the
request's `advanced` and `creative` flags on native and wasm alike.
Vanilla enchantment descriptions, including those on enchanted books, retain
their root `TextContent::Translatable { key, .. }`. Match that key (for example,
`enchantment.minecraft.sharpness`), not the displayed name, level, or color.

## Drawing a second world camera

Native internals mods can request extra world cameras through `world_views` and
consume their textures in `world_view_drawn`. These are raw renderer callbacks, not
portable API callbacks: wasm mods cannot use them. Construct a `WorldView` with a
stable ID, camera, yaw, pitch, field of view and pixel size, and push it to the
`WorldViews` the hook hands you. The hook is asked once a frame. Up to eight
views with distinct IDs are taken, with sides limited to 4096 pixels and the
device limit, and 4 x 4096 x 4096 pixels in all. `push` refuses any other view
with a `WorldViewRefusal` naming it and the reason, and logs each refusal once.
Each view culls the world with its own frustum, and the player's view keeps its
own culling. Entities and block entities are built for what the camera or any
view sees; entity occlusion culling (the `entityCulling` setting) proves an entity
hidden from the player's camera only, so it is off for a frame with views, and an
entity behind a hill from the player still reaches a view that sees it. These
callbacks have no Wire form.

For a view seen in another view, such as a portal seen through a portal, push the
outer view first and add `with_parent(outer_id)` to the inner one. Views nest up to
depth 5; depth 6 is refused. The inner view is drawn first, so in the outer view's
`world_view_drawn` its picture is this frame's: draw it into the outer view's
target there. `WorldViewDrawn` carries each view's `parent` and `nesting_depth`.

For a view through a portal, add `with_clip_plane([x, y, z, d])`: nothing behind
that world-space plane is drawn, and the camera must be behind it. In such a view
an entity wins the depth test against terrain up to 1/1024 of its distance in
front of it (about 1 block at 1024 blocks). For a mirror,
give the camera reflected through it and add `with_mirror()`: the picture is
flipped left to right, with face culling flipped to match, so the same faces are
culled as in an unmirrored view. A clip plane with a zero normal, a value that is
not finite, or the camera in front of it refuses the view.

A view draws the player's level unless it names another with
`with_level(Some(level))`, a `LevelRef` from `host.level(dimension)`; `None` is the
player's level, and so is a handle naming it. Viewed levels are not drawn yet: a view
of any other level (one the server keeps open for viewing, a handle from an earlier
epoch, a level closed or never open) is refused as `WorldViewRefusalReason::Level`
and logged once. The handle is resolved once a frame, so a stale one is refused from
the next frame on.

Use `world_view_drawn` to copy a still to your own buffer or draw into the supplied
target with its encoder. Copy a picture you need to preserve: targets are reused
while requested and retired when absent. Views draw before `world_drawn`, where
you can composite their pictures into the player's frame. Extra cameras increase
rendering work and do not include particles, weather, hands or shader-pack passes.
Extra views also omit the sun, moon, stars, clouds and block outline. Translucent
terrain uses the player camera's sorting. See the generated `world_views` hook notes
for the rendering contract.

Views draw the player's own level for now. The other levels a server opens for the
client (viewed levels) are meshed only while a view names one, after the player's own
level's work and on one worker at a time, and their meshes stay 5 seconds after no view
names the level. Once views of a viewed level are drawn, such a view draws the sections
meshed so far and nothing where a section is not meshed yet.

## Matching the frame's sky and light

A pass in `world_drawn` that draws its own picture (a map, a hologram, a view's
composite) can sit in the frame's light with `WorldDrawn::look` (`WorldLook`): the
fog's colour and distances as the terrain mixes them, unit world directions to the
sun, the moon and the body lighting the frame, `sunAngle`, the sky, ambient and block
light colours, the sunrise glow, star brightness, under water and water's fog colour,
and night vision. With a shader pack on, the directions follow the pack's
`sunPathRotation` and `shader_pack` is set: the pack's own sky, fog, lighting and
tonemap are its shader code and are already in `color`, so copy from `color` to match
them exactly, and use `look` for what the pack's uniforms were built from.

## Muffling sounds

`ClientHost::muffle_sounds(amount, lowpass, ticks, exempt)` temporarily muffles the sound output, for example to accompany ringing ears after a nearby blast:

- `amount`, from 0 to 1, is the fraction of every sound's volume taken away.
- `lowpass`, from 0 to 1, dulls the output, from untouched to a 300 Hz low-pass.
- Both effects start at full strength and fade linearly over `ticks` client ticks. The fade waits while the game is paused.
- The sound events in `exempt` remain clear, including ones already playing. Use this for the mod's ringing sound. Other sounds already playing, music and interface clicks are muffled too.

A new call replaces the current muffle; `ticks` of 0 ends it immediately. The effect belongs to the sound output, so every splitscreen player hears it. The call refuses an amount or strength outside 0 to 1, more than 64 exempt events, or an exempt event that is not a namespaced sound ID, without changing the output. Server code cannot call this client-only service.

Play the accompanying sound through `ClientHost::play_local_sound` with a `SoundPlay`.

## Playing your own samples

`ClientHost::play_local_pcm(&sound, samples, rate, channels, start, after)` plays audio the mod decoded or made
itself, such as an emote's song, a recorded clip or a synthesised tune, for this player only:

- `samples` is signed 16-bit PCM, interleaved, with `channels` (1 or 2) samples to a frame at `rate` frames a
  second (8000 to 192000). The host copies them during the call.
- Playback begins `start` frames in, so a player who joins a song already under way hears it from the right
  place.
- `sound` is a `SoundPlay`, as for `play_local_sound`: its source's volume slider, volume, pitch, position,
  attenuation and looping apply, and the sound pauses with the game. Its event is not loaded. It names the
  play for `play_sound` hooks and `muffle_sounds` exemptions, and its subtitle shows if the resources have one.

The returned `SoundRef` works as `play_local_sound`'s does (`state`, `stop`, `fade`). `SoundRef::set_position`
moves a playing sound from the next audio update, so a song played at its player can follow them. Keep the
handle while the song plays and move it from a tick hook (`dancer_position` stands for however the mod finds
the player):

```rust
static SONG: std::sync::Mutex<Option<mod_api::SoundRef>> = std::sync::Mutex::new(None);

// When the emote starts: 48 kHz mono, joined 2.5 seconds in.
let play = mod_api::SoundPlay::at("mymod:emote.song", mod_api::SoundSource::Players, dancer_position(&*host));
*SONG.lock().unwrap() = host.play_local_pcm(&play, &pcm, 48_000, 1, 120_000, 0);

// Each client tick, while it plays:
fn client_tick_post(&self, host: &mut dyn mod_api::ClientHost, _: &mut mod_api::ClientTickPostEvent) {
    let mut song = SONG.lock().unwrap();
    if let Some(sound) = *song {
        // false once the song is over: let the handle go.
        if !sound.set_position(&*host, dancer_position(&*host)) {
            *song = None;
        }
    }
}
```

The call returns `None` and plays nothing for an empty clip, a `start` past its end, a rate or channel count
out of range, or more than 28 800 000 samples (ten minutes of mono at 48 kHz). Server code cannot call it.

## Showing another time or weather

`sky_environment` lets a client mod show the local player another time of day or weather, for example noon
while building, a clear sky for a recording, or night on a server where it is day. It is asked whenever the
frame reads the sky, with the frame's `partial` tick and the values the frame would draw:

| `SkyEnvironmentEvent` field | Meaning |
| --- | --- |
| `partial` | Ticks past the last tick, from 0 to 1 |
| `day_time` | The level's time of day (or PvP & HUD's view time); write it to show another |
| `rain` | The rain level from 0 to 1; write it to show more or less rain |
| `thunder` | The thunder level from 0 to 1; write it to show more or less thunder |

Only the picture follows: the sun, moon and stars, the sky's and fog's colour, the sky's light, clouds, and
falling rain or snow. The level's own clock and weather, gameplay (crops, mobs, beds) and the server keep
theirs, and nothing is sent. Rain and thunder count only in a dimension with weather. A level that is not finite
keeps the level's value, and the others are clamped to 0..=1. The hook may run more than once a frame, so
give the same answer each time. Only a client of a world (single player's included) asks it, never the game
that ticks the world. NeoForge has no counterpart; Java mods change `ClientLevel` with a mixin.
