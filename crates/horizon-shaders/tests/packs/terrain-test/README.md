# terrain-test

Horizon's own shader pack for testing gbuffers terrain (written for Horizon, no other pack's code).
`gbuffers_terrain` draws blocks from the atlas with their vertex colour and lightmap, tinted by
the lightmap coordinate's sky light (cooler where the sky is hidden); `mc_midTexCoord` marks each
face's centre with a dot, and any stage but solid terrain (translucent terrain falls back to it)
is red. `gbuffers_skybasic` draws the sky in its vertex colour, a little greener, with magenta
stars and a dark red void. `composite` puts the depth buffer's view-space distance into the blue
channel of the far half. Copy the folder into `shaderpacks/` and run the game with
`HORIZON_SHADER_PACK=terrain-test`.
