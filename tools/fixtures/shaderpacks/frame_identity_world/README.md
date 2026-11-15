# Frame identity: world

A test fixture of `tools/test_frame_identity_parity.py` (sessions `world_pack` and `world_pack_timed`), not a look
and not offered to players: a shader API 0.2 pack with the stages that draw the world around terrain, so a change to
how they reach the pack is held to the same pixels. Each part changes the picture plainly:

- `[sky]` tints vanilla's sky and draws its own clouds, stars and sun (`clouds`, `stars` and `sun` off), so a sky
  that falls back to the renderer's shows. With GPU pass timing on (`world_pack_timed`) the sky draws in a pass of
  its own.
- `[weather]` turns rain and snow orange.
- `[material]` draws water as half the water mirror (`reflection`) plus what lies behind it (`opaque_color` and
  `opaque_depth`), and stores solid terrain's normal and sky light with `set_surface`.
- `[[post]]` tints solid terrain by what `set_surface` stored (`surface_at`), and grades the whole picture, the
  End's sky included, which no other stage reaches.

Check it with `horizon-shader check tools/fixtures/shaderpacks/frame_identity_world`.
