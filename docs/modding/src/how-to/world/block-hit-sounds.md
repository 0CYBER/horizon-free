# Block hit sounds

Use a block part's `play_hit_sound` to handle mining hit sounds on the client.
The event supplies the block state, position and face. Return `Flow::Cancel` after handling the sound through the supplied `ClientHost` to skip the
vanilla sound. Other blocks keep their normal sound.

The default uses the state's hit sound with volume `(volume + 1) / 8` and pitch
`pitch * 0.5`. Sculk growth sounds are outside this hook.
