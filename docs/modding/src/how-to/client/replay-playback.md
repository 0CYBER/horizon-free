# Replay playback

A client mod plays the recordings `/studio record` saves (`.mcpr` files in the game folder's `recordings/`) and
drives their clock, as a replay editor does. Native and wasm mods use the same calls; a wasm mod needs no file
access, because a recording is named by its **id**: its file name without `.mcpr` (`my_trip_1712000000` for
`recordings/my_trip_1712000000.mcpr`). An id is never a path. One that is empty, longer than 250 bytes, holds a
separator, a drive, `..` or a control character, ends in a dot or a space, or is a Windows device name (`CON`,
`NUL`, `COM1`...) is `ReplayError::InvalidId`, refused before any file is looked at. One that names no regular file
directly in `recordings/` is `NotFound`, and so is a link, or a `recordings/` folder that is itself a link. A replay
opened from a file elsewhere (`/studio replay open <path>`) has the empty id.

| Call | Access | What it does |
|---|---|---|
| `replay_state()` | client setup | The open replay as its clock stands: id, `standalone`, `position_ms` (0 to `duration_ms`), `duration_ms`, `speed`, `paused`, `ended`. None while no replay plays; while a requested one is still opening, the replay playing (if any). |
| `replay_open(id)` | world | Accepts a request and returns its id: opens the file once, through the recordings folder's handle, and checks it starts with a zip signature (`Unreadable` if not). The recording is read off the game's thread, from that open file, and opened at a frame's end; an archive found broken there fails through `replay_open_failed` with `Unreadable`. |
| `replay_close()` | world | Closes the replay at the end of the frame, as Escape does. |
| `replay_pause()`, `replay_resume()` | world | Stop and start the clock at once. |
| `replay_set_speed(speed)` | world | Sets the speed at once, `REPLAY_MIN_SPEED` (0.01) to `REPLAY_MAX_SPEED` (20); anything else, NaN included, is `InvalidSpeed`. |
| `replay_seek(ms)` | world | Moves the clock at the end of the frame, with the recording's packets up to there processed (see below). Only forward for a replay that doesn't own its session (`BackwardSeek`). |

`Ok(request)` from `replay_open` means the request is accepted; `request` is a non-zero id never given twice while
the game runs (once every id has been given, `replay_open` is `Unavailable`). Exactly one of `replay_opened` or `replay_open_failed` follows with that id, whatever happens; only
the game closing first promises neither. One thread reads recordings at a time: a later `replay_open` replaces a
request still waiting or being read, which fails with `Superseded`, and the replaced reading is cancelled. A read
request waiting for a world to be left is past that: a later `replay_open` is `WorldBusy` and the first goes on.
While a replay plays, its edited Studio camera path is saved before another replaces it. If that save fails, or
if the requested copy was read from the same archive and its markers or mod data were saved after the request,
the replay playing stays and the request fails with `Unavailable`.

A replay never displaces a session it would end: `replay_open` is `WorldBusy` while a world is being joined, a
multiplayer server is connected, or the single-player world is open to LAN or has splitscreen players (leave it with
`disconnect` first). A request whose session changes before the recording is ready (a world joined, left or opened,
the replay playing closed or replaced, or another world entry taking its place) fails with `WorldBusy`; a dimension
change inside the replay playing is no such change. From a plain single-player world, the world is saved and left as
Save and Quit does, and the replay opens on the title; if the person cancels that wait with Escape, the request fails
with `Unavailable` and the world stays left. A recording that turns out damaged once read fails with `Unreadable`.

Close and seek run at the end of the frame, outside the calling hook; one of them a frame, the last one winning, and
a request for a replay that has closed or been replaced since is dropped. While no replay plays, the controls
(close, pause, resume, a valid speed, seek) are `NotOpen` and `replay_state()` is None, so seek to a start point from
`replay_opened`, not right after `replay_open`. `replay_set_speed` checks the speed first: an invalid one is
`InvalidSpeed` either way.

After a seek, the recording's packets up to the position the seek applied have been processed in order (a packet
the game cannot decode or apply is skipped), and the next `replay_advanced` reports exactly that position. A standalone replay sought back starts again from a fresh
session; what the game keeps outside that session may carry over (today the difficulty shown, and the backing state
of a screen a packet opened). State that came only from the client's own simulation between packets (an entity
moving between its updates, particles) is not promised either way. How the game gets there, and how long it takes,
is not part of the contract: today a seek forward costs only the packets between the clock and the target, so an
exporter that pauses and seeks one later position per frame pays for each frame's packets once, while a seek back in a
standalone replay replays every packet from the start. A standalone replay also replays from the start when the
packets in between move the player relative to where it is (a relative teleport), or when reading them fails. When
a rebuild fails (sought back or after a failed forward read), or a read fails while playing or while a replay that
doesn't own its session seeks forward, the replay closes (`replay_closed`); a failed seek is never reported by
`replay_advanced`. A read that fails on the first frame of a mod's open fails that open instead: one
`replay_open_failed`, and no `replay_opened` or `replay_closed`.

The clock stays within the recording: `position_ms` never passes `duration_ms` (the recorded length, or the last
packet's time when that is later), and `ended` is true exactly when the clock is at `duration_ms`, whether it played
there or was sought there, paused or not.

Hooks follow playback, whoever drives it (a mod, the replays list or `/studio replay`):

- `replay_opened(recording, duration_ms, standalone, request)`: the replay is open and its first frame set up;
  `request` is the id `replay_open` returned, or 0 for a replay opened another way.
- `replay_open_failed(request, recording, error)`: an accepted `replay_open` opened nothing.
- `replay_advanced(position_ms)`: a once-a-frame sample of the clock, on a frame it moved. After a seek, the next one
  is exactly the seek's position, on the frame that first shows it; the clock runs on from the frame after.
- `replay_closed(recording)`: the replay is gone; before `replay_opened` of one opened over it.

While a replay plays, the recording's world is not the player's: movement, attack, use, drop, inventory and hotbar
keys do nothing, the wheel and a controller's bumpers don't step the hotbar or the spectator menu, and losing the
window's focus doesn't pause, with or without Studio's free camera. A mod still hears every key (`input_key`, its
key mappings), mouse button (`mouse_button_pre`/`_post`), the wheel and the bumpers (`mouse_scrolling`, the gameplay route:
not while a screen or prompt, chat included, takes the wheel), and its screens and prompts take input as usual, so a replay editor can run on a mod's own camera (`camera_pose`). Escape
closes the replay and P opens the pause menu.

## Markers and mod data

These calls act on the replay playing in this window. They take neither a path nor a recording id.
Use `replay_opened` to know when your requested recording is ready.

| Call | Access | What it does |
|---|---|---|
| `replay_markers()` | client setup | Returns the replay's bookmarks in timeline order. |
| `replay_add_marker(label)` | world | Saves a bookmark at the current playhead and returns it. |
| `replay_replace_markers(markers)` | world | Replaces the whole list. Create entries with `ReplayMarker::new(time_ms, label)`. Ties keep their input order; an empty list clears it. |
| `replay_read_mod_data()` | world | Reads this mod's bytes. `Ok(None)` means there is no entry. |
| `replay_write_mod_data(data)` | world | Replaces this mod's bytes with `Some(bytes)`, or removes them with `None`. `Some(vec![])` is a present but empty entry. |

Markers are shared platform data and still use ReplayMod's `markers.json`. Mod data is separate: the host
uses the calling package's id, so two mods can save data in the same replay without replacing one another's
entries. Data travels with the `.mcpr`; it isn't private to the player who saved it. The ZIP entry is
`horizon/mod-data/<package id>.bin`. Only `replay_studio` falls back to the old `camera.json`, and only
when its new entry is absent. Updating or removing that entry also removes `camera.json` so outdated
data cannot reappear. The built-in editor reads and saves to the same entry; an update from `replay_studio`
is refused while the editor has unsaved camera changes.

Each mod's entry and the whole serialized marker list have a 1 MiB limit (`REPLAY_EDIT_MAX_BYTES`).
Saves are synchronous and atomic. They copy the rest of the archive without changing the recording's packets
or metadata, so save at editing milestones rather than every frame. ZIPs over 1 GiB plus 64 MiB cannot be
edited. `ReplayEditError::TooLarge` and `Stale` leave the recording unchanged; `Stale` also covers the
recording being replaced externally while it is open. `NotOpen` means there is no replay, `Unavailable`
means there is no editable archive for this replay, `NoCallingMod` means there is no package to bind the data to,
and `Io` means the archive could not be read or written.

`ReplayState`, `ReplayError`, `ReplayMarker`, `ReplayEditError` and the hooks' events are `#[non_exhaustive]` for Rust callers only. On the wire every
field and variant is encoded and its layout hashed ([wire types](../../concepts/wire-types.md)), so they stay as they
are: adding one would break wasm mods built against this version.
