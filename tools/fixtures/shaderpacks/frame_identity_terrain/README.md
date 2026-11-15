# Frame identity: terrain

A test fixture of `tools/test_frame_identity_parity.py` (sessions `terrain_pack` and `terrain_pack_hdr`), not a
look and not offered to players: a shader API 0.2 pack with the stages that read terrain, so a change to how terrain
reaches them is held to the same pixels. `[material]` lights terrain with vanilla's light at a third plus the sun (the
moon by night) through the `[shadow]` map, on the LabPBR normal map's normals and ambient occlusion;
`[displacement]` turns the normal maps' heights into geometry near the camera. The heights come from a resource
pack the harness writes (`relief_pack`). Check it with
`horizon-shader check tools/fixtures/shaderpacks/frame_identity_terrain`.
