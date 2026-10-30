# post-test

Horizon's own shader pack for testing post-processing (written for Horizon, no other pack's code).
`composite` tints the world warm and stores linear depth in colortex2, `composite1` shows that
depth as fog bands on the right half with a little noise, and `final` adds a vignette. Copy the
folder into `shaderpacks/` and run the game with `HORIZON_SHADER_PACK=post-test`.
